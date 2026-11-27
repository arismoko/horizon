//! The game's item places for native mods (`mod_api::ItemPlace`, arismoko/horizon#41 §7): a
//! player's slots, hands and cursor, entities' slots, item entities and the guest storage a
//! menu slot shows (ender chest, crafting grid, a mod's chest menu). Mods read and write
//! them through `ItemPlace`'s `with_stack`, `insert` and `extract`, which come here for these
//! places; a block entity's slots, a stack inside an item and a mod's own places go through their handlers.
use crate::game::ServerGame;
use mod_api::__private::game::*;
use mod_api::{
    CraftedByError, EquipmentSlot, ItemComponentEditor, ItemPlace, ItemPlaceEditError,
    ItemStackView, LivingRef, MenuSlotBacking, TransactionContext,
};
use sunrise_gameplay_common::game::api_values::component_edit::{
    GearEdits, prepare_component_edit,
};
use sunrise_gameplay_common::game::native_server::PlayerRead;
use sunrise_gameplay_common::game::transactions::{After, DeferredEffect};
use {
    sunrise_items::items::ItemStack, sunrise_items::items::Items,
    sunrise_sim::first_person_sim::Hand, sunrise_sim::menu::Menu, sunrise_sim::mobs::finalize,
    sunrise_sim::mobs::finalize::EquipmentRead,
};

/// An occupied inventory cell, addressed within the resolved player's own storage.
pub(in crate::game) enum PlayerStackSlot {
    Inventory(usize),
    Armor(usize),
    Offhand,
}

impl PlayerStackSlot {
    pub(in crate::game::server_side) fn borrow<'a>(
        place: &ItemPlace,
        inventory: &'a [Option<ItemStack>],
        armor: &'a [Option<ItemStack>; 4],
        selected: usize,
        offhand: &'a Option<ItemStack>,
    ) -> Option<(Self, EquipmentRead<'a>)> {
        let slot = match place {
            ItemPlace::PlayerSlot {
                slot: slot @ 100..=103,
                ..
            } => Self::Armor(103 - *slot as usize),
            ItemPlace::PlayerSlot { slot: -106, .. }
            | ItemPlace::PlayerHand {
                hand: Hand::Off, ..
            } => Self::Offhand,
            ItemPlace::PlayerSlot {
                slot: slot @ 0.., ..
            } => Self::Inventory(*slot as usize),
            ItemPlace::PlayerHand {
                hand: Hand::Main, ..
            } => Self::Inventory(selected),
            _ => return None,
        };
        let stack = match slot {
            Self::Inventory(index) => inventory.get(index)?,
            Self::Armor(index) => &armor[index],
            Self::Offhand => offhand,
        }
        .as_ref()
        .filter(|stack| stack.count > 0)?;
        Some((slot, EquipmentRead::resident(stack)))
    }

    /// The same cell after preparation and journaling, neither of which changes the row.
    pub(in crate::game::server_side) fn stack_mut<'a>(
        self,
        inventory: &'a mut [Option<ItemStack>],
        armor: &'a mut [Option<ItemStack>; 4],
        offhand: &'a mut Option<ItemStack>,
    ) -> &'a mut ItemStack {
        match self {
            Self::Inventory(index) => &mut inventory[index],
            Self::Armor(index) => &mut armor[index],
            Self::Offhand => offhand,
        }
        .as_mut()
        .expect("prepared player slot")
    }
}

impl ServerGame<'_> {
    /// The player a place belongs to, if a player's: a cursor's only while its menu is open.
    pub(in crate::game::server_side) fn place_player(&self, place: &ItemPlace) -> Option<u128> {
        match place {
            ItemPlace::PlayerSlot { player, .. } | ItemPlace::PlayerHand { player, .. } => {
                Some(*player)
            }
            ItemPlace::PlayerCursor { menu } => self
                .native_menu_guest(*menu)
                .filter(|guest| !Self::menu_cursor_retired(guest))
                .map(|_| menu.fields().player.uuid()),
            _ => None,
        }
    }
    /// The inventory of entity `id` in the level in place, if it has one: a chest or hopper
    /// minecart's (the open menu's copy while one shows it, as `cart_slots`), a chest boat's, or
    /// a chested donkey's, mule's or llama's (`AbstractHorse.inventory`, `getInventoryColumns`
    /// rows of three). With where it stands, for what a split throws.
    pub fn edit_entity_slots<R>(
        &mut self,
        id: u32,
        edit: impl FnOnce(&mut [Option<ItemStack>], [f64; 3]) -> R,
    ) -> Option<R> {
        let key = (i32::MAX - 15, super::minecarts::CART_KEY_Y, id as i32);
        let at = |p: &sunrise_core::geometry::Vec3| [p.x, p.y, p.z];
        let container = |c: &super::minecarts::Minecart| c.id == id && c.kind.slots() > 0;
        let position = if let Some(c) = self.common.carts.iter().find(|c| container(c)) {
            at(&c.position)
        } else if let Some(b) = self.common.boats.iter().find(|b| b.id == id && b.chest) {
            at(&b.position)
        } else {
            let m = self
                .common
                .mobs
                .iter_mut()
                .find(|m| m.id == id && m.extra.chested)?;
            let size = sunrise_gameplay_common::mount_rules::mount_columns(m) * 3;
            if m.extra.items.len() < size {
                let dim = self.common.dim;
                if let Some(level) = self.common.transactions.level_frame(dim) {
                    level.keep_mob_length(dim, id, m.extra.items.len());
                }
                m.extra.items.resize(size, None);
            }
            return Some(edit(&mut m.extra.items[..size], at(&m.position)));
        };
        if self.common.carts.iter().any(container) {
            return self.edit_cart_slots(id, |slots| edit(slots, position));
        }
        self.unpack_loot(
            sunrise_gameplay_common::game::container_loot_level::LootTarget::Boat(id),
            None,
        );
        let i = self
            .common
            .boats
            .iter()
            .position(|b| b.id == id && b.chest)?;
        if let Some(super::BlockEntity::Chest(open)) =
            self.common.world.block_entities.get_mut(&key)
        {
            return Some(edit(open, position));
        }
        Some(
            self.common
                .boats
                .update(i, |b| edit(&mut b.items, position)),
        )
    }
    /// Whether this game simulates a world: a client's copy has no places to write.
    pub(in crate::game::server_side) fn places_simulated(&self) -> bool {
        self.common.is_logical_server()
    }
    /// The slot of the player in place that `place` names.
    pub(in crate::game::server_side) fn player_place(
        &mut self,
        place: &ItemPlace,
    ) -> Option<&mut Option<ItemStack>> {
        match place {
            ItemPlace::PlayerSlot { slot, .. } => self.native_carried_slot(*slot),
            ItemPlace::PlayerHand {
                hand: Hand::Main, ..
            } => self.common.inventory.get_mut(self.common.selected),
            ItemPlace::PlayerHand {
                hand: Hand::Off, ..
            } => Some(&mut self.common.offhand),
            ItemPlace::PlayerCursor { .. } => Some(&mut self.common.carried),
            _ => None,
        }
    }
    /// Player place insert and extract use the command/host slot writer (and its journal).
    pub(in crate::game::server_side) fn write_player_place(
        &mut self,
        place: &ItemPlace,
        stack: Option<ItemStack>,
    ) {
        use super::commands::items::Slot;
        let slot = match place {
            ItemPlace::PlayerSlot {
                slot: slot @ 0..=35,
                ..
            } => Slot::Inv(*slot as usize),
            ItemPlace::PlayerSlot {
                slot: slot @ 100..=103,
                ..
            } => Slot::Armor(103 - *slot as usize),
            ItemPlace::PlayerSlot { slot: -106, .. } => Slot::Offhand,
            ItemPlace::PlayerHand {
                hand: Hand::Main, ..
            } => Slot::Inv(self.common.selected),
            ItemPlace::PlayerHand {
                hand: Hand::Off, ..
            } => Slot::Offhand,
            ItemPlace::PlayerCursor { .. } => {
                self.common.carried = stack;
                return;
            }
            _ => return,
        };
        self.slot_set(&slot, stack);
    }
    /// How many slots the inventory of entity `entity` in the level in place has, as
    /// `entity_slots` reaches them: none for an entity without one, and on a client.
    pub(in crate::game) fn entity_container_size(&self, entity: u32) -> Option<usize> {
        self.entity_automation_size(entity).or_else(|| {
            let m = self
                .common
                .mobs
                .iter()
                .find(|m| m.id == entity && m.extra.chested)?;
            Some(sunrise_gameplay_common::mount_rules::mount_columns(m) * 3)
        })
    }
    /// How many slots the container entity `entity` of the level in place shows automation
    /// (`EntitySelector.CONTAINER_ENTITY_SELECTOR`: chest and hopper minecarts, chest boats; a
    /// chested mount is not one): none for any other entity, and on a client.
    pub(in crate::game) fn entity_automation_size(&self, entity: u32) -> Option<usize> {
        if !self.common.is_logical_server() {
            return None;
        }
        if let Some(c) = self.common.carts.iter().find(|c| c.id == entity) {
            let container = c.kind.slots() > 0;
            return container.then_some(c.items.len());
        }
        let b = self.common.boats.iter().find(|b| b.id == entity)?;
        b.chest.then_some(b.items.len())
    }
    /// How many equipment slots living entity `entity`'s [`ENTITY_ITEMS`](mod_api::ENTITY_ITEMS)
    /// answers (#932, NeoForge's `LivingEntityEquipmentWrapper`): a mob's or an armor stand's
    /// seven (main hand, off hand, feet, legs, chest, head and body, by `EquipmentSlot`'s
    /// ordinal; the saddle is not automation's to move), none for any other entity, and on a
    /// client.
    pub(in crate::game) fn entity_equipment_size(&self, entity: u32) -> Option<usize> {
        // The handler must not exist on a client: the same gate every read
        // and write of a slot answers (`equipment_stack`).
        if !self.places_simulated() {
            return None;
        }
        (self
            .common
            .mobs
            .iter()
            .any(|m| m.id == entity && !m.removed)
            || self.common.armor_stands.iter().any(|a| a.id == entity))
        .then_some(7)
    }
    /// Lend a native place's stack without encoding components. Reading a chest vehicle's
    /// `EntitySlot` unpacks its pending loot table without a player
    /// (`ContainerEntity.getChestVehicleItem`).
    pub(in crate::game) fn game_stack(
        &mut self,
        place: &ItemPlace,
        read: &mut dyn for<'a> FnMut(ItemStackView<'a>),
    ) -> Option<()> {
        sunrise_gameplay_common::game::native_equipment_read::lend_equipment(
            self.game_stack_source(place)?,
            read,
        )
    }

    /// Borrow the stack in a native place without initializing a player. Reading a chest
    /// vehicle's `EntitySlot` unpacks its pending loot table without a player; equipment
    /// overlays are only applied when the caller projects a complete stack.
    pub(in crate::game::server_side) fn game_stack_source(
        &mut self,
        place: &ItemPlace,
    ) -> Option<EquipmentRead<'_>> {
        if let ItemPlace::Scratch(slot) = place {
            return self.common.scratch_held(*slot).map(EquipmentRead::resident);
        }
        if !self.places_simulated() {
            return None;
        }
        let stack = match place {
            ItemPlace::ItemEntity { id } => {
                return self
                    .common
                    .dropped
                    .iter()
                    .find(|entity| entity.id == *id)
                    .map(|entity| EquipmentRead::resident(&entity.stack));
            }
            ItemPlace::EntitySlot { entity, slot } => {
                if self
                    .common
                    .carts
                    .iter()
                    .any(|c| c.id == *entity && c.kind.slots() > 0)
                {
                    self.unpack_loot(
                        sunrise_gameplay_common::game::container_loot_level::LootTarget::Cart(
                            *entity,
                        ),
                        None,
                    );
                } else if self.common.boats.iter().any(|b| b.id == *entity && b.chest) {
                    self.unpack_loot(
                        sunrise_gameplay_common::game::container_loot_level::LootTarget::Boat(
                            *entity,
                        ),
                        None,
                    );
                }
                let vehicle = self
                    .common
                    .carts
                    .iter()
                    .find(|c| c.id == *entity && c.kind.slots() > 0)
                    .map(|cart| cart.items.as_slice())
                    .or_else(|| {
                        self.common
                            .boats
                            .iter()
                            .find(|b| b.id == *entity && b.chest)
                            .map(|boat| boat.items.as_slice())
                    });
                let slots = if let Some(slots) = vehicle {
                    // The placeholder owns the live slots while a vehicle's menu is open.
                    let key = (i32::MAX - 15, super::minecarts::CART_KEY_Y, *entity as i32);
                    match self.common.world.block_entities.get(&key) {
                        Some(super::BlockEntity::Chest(open)) => open.as_slice(),
                        _ => slots,
                    }
                } else {
                    let mob = self
                        .common
                        .mobs
                        .iter()
                        .find(|m| m.id == *entity && m.extra.chested)?;
                    let size = sunrise_gameplay_common::mount_rules::mount_columns(mob) * 3;
                    &mob.extra.items[..size.min(mob.extra.items.len())]
                };
                slots.get(*slot)?.as_ref()
            }
            ItemPlace::Equipment { entity, slot } => {
                return self
                    .equipment_stack_source(*entity, *slot)?
                    .filter(|stack| stack.base().count > 0);
            }
            ItemPlace::PlayerSlot { player, .. } | ItemPlace::PlayerHand { player, .. } => {
                let (inventory, armor, selected, offhand) =
                    match self.native_item_place_player(*player)? {
                        PlayerRead::Acting { .. } => (
                            &self.common.inventory,
                            &self.common.armor,
                            self.common.selected,
                            &self.common.offhand,
                        ),
                        PlayerRead::Guest(guest) => (
                            &guest.inventory,
                            &guest.armor,
                            guest.selected,
                            &guest.offhand,
                        ),
                    };
                match place {
                    ItemPlace::PlayerSlot { slot, .. } => match *slot {
                        100..=103 => armor[103 - *slot as usize].as_ref(),
                        -106 => offhand.as_ref(),
                        0.. => inventory.get(*slot as usize)?.as_ref(),
                        _ => None,
                    },
                    ItemPlace::PlayerHand {
                        hand: Hand::Main, ..
                    } => inventory.get(selected)?.as_ref(),
                    ItemPlace::PlayerHand {
                        hand: Hand::Off, ..
                    } => offhand.as_ref(),
                    _ => None,
                }
            }
            ItemPlace::PlayerCursor { menu } => {
                let guest = self.native_menu_guest(*menu)?;
                if Self::menu_cursor_retired(guest) {
                    return None;
                }
                if self.server.lan_acting == Some(guest.conn) {
                    self.common.carried.as_ref()
                } else {
                    guest.carried.as_ref()
                }
                .filter(|stack| stack.count > 0 && stack.item.as_ref() != "minecraft:air")
            }
            _ => None,
        }?;
        (stack.count > 0).then_some(EquipmentRead::resident(stack))
    }
    /// Edit the resolved slot without going through an equipment setter or stopping item use.
    pub(in crate::game) fn game_edit_stack(
        &mut self,
        place: &ItemPlace,
        tx: &TransactionContext,
        edit: &mut dyn for<'a> FnMut(&mut ItemComponentEditor<'a>),
    ) -> Result<(), ItemPlaceEditError> {
        self.edit_resolved_stack(
            place,
            tx,
            |read, items| prepare_component_edit(read, items, edit),
            |held, patch| patch.apply(held),
        )
    }

    /// Prepare against the occupied slot and apply to that same cell, without lending a Host.
    /// Preparation can refuse or leave the slot unchanged; only a prepared write gets journaled
    /// and published.
    pub(in crate::game::server_side) fn edit_resolved_stack<P>(
        &mut self,
        place: &ItemPlace,
        tx: &TransactionContext,
        prepare: impl FnOnce(
            EquipmentRead<'_>,
            &Items,
        ) -> Result<Option<(P, GearEdits)>, ItemPlaceEditError>,
        apply: impl FnOnce(&mut ItemStack, P),
    ) -> Result<(), ItemPlaceEditError> {
        if !self.transaction_context_is_current(tx) {
            return Err(ItemPlaceEditError::InvalidContext);
        }
        if !self.places_simulated() {
            return Err(ItemPlaceEditError::Unavailable);
        }
        match *place {
            ItemPlace::Equipment { entity, slot } => {
                let items = server_world_items!(self);
                let prepare = |read: Option<EquipmentRead<'_>>| {
                    let read = read
                        .filter(|read| read.base().count > 0)
                        .ok_or(ItemPlaceEditError::NotHeld)?;
                    prepare(read, items)
                };
                let index = slot as usize;
                let (equipment, mob_gear, prepared) = if let Some(mob) = self
                    .common
                    .mobs
                    .iter_mut()
                    .find(|m| m.id == entity && !m.removed)
                {
                    let prepared = prepare(finalize::equipment_slot(mob, slot))?;
                    (
                        &mut mob.extra.equipment,
                        Some(&mut mob.extra.gear),
                        prepared,
                    )
                } else if let Some(stand) =
                    self.common.armor_stands.iter_mut().find(|a| a.id == entity)
                {
                    let prepared =
                        prepare(stand.equipment.get(index).map(EquipmentRead::resident))?;
                    (&mut stand.equipment, None, prepared)
                } else {
                    return Err(ItemPlaceEditError::NotHeld);
                };
                let Some((patch, edits)) = prepared else {
                    return Ok(());
                };
                let dim = self.common.dim;
                if let Some(level) = self.common.transactions.level_frame(dim) {
                    level.keep_equipment(dim, entity, equipment, mob_gear.as_deref());
                }
                let held = equipment[index].as_mut().expect("prepared equipment slot");
                apply(held, patch);
                if let Some(overlay) = mob_gear {
                    // Only hands and armor have a separate gear overlay. An edit
                    // to one of its components must not rewrite the other one.
                    if index <= finalize::HEAD && (edits.enchantments || edits.trim) {
                        let enchantments = edits.enchantments.then(|| held.enchantments());
                        let trim = edits
                            .trim
                            .then(|| held.data().and_then(|data| data.trim.as_ref()));
                        if enchantments.is_some_and(|levels| !levels.is_empty())
                            || trim.flatten().is_some()
                        {
                            overlay.get_or_insert_with(Box::default);
                        }
                        if let Some(gear) = overlay.as_deref_mut() {
                            let index = finalize::gear_slot(index);
                            if let Some(levels) = enchantments {
                                levels.clone_into(&mut gear.slots[index]);
                            }
                            if let Some(trim) = trim {
                                gear.trims[index] = trim.cloned();
                            }
                            if gear.is_empty() {
                                *overlay = None;
                            }
                        }
                    }
                    self.after_commit(After::EquipmentSync(entity));
                }
            }
            ItemPlace::PlayerSlot { player, .. } | ItemPlace::PlayerHand { player, .. } => {
                let (parked, inventory, armor, selected, offhand) = match self
                    .native_item_place_player(player)
                    .ok_or(ItemPlaceEditError::NotHeld)?
                {
                    PlayerRead::Acting { .. } => (
                        None,
                        &self.common.inventory,
                        &self.common.armor,
                        self.common.selected,
                        &self.common.offhand,
                    ),
                    PlayerRead::Guest(guest) => (
                        Some(
                            self.guest_index(guest.conn)
                                .ok_or(ItemPlaceEditError::NotHeld)?,
                        ),
                        &guest.inventory,
                        &guest.armor,
                        guest.selected,
                        &guest.offhand,
                    ),
                };
                let (slot, read) =
                    PlayerStackSlot::borrow(place, inventory, armor, selected, offhand)
                        .ok_or(ItemPlaceEditError::NotHeld)?;
                let Some((patch, _)) = prepare(read, world_items!(self))? else {
                    return Ok(());
                };
                if let Some(index) = parked {
                    self.journal_parked_player(index);
                    let host = self.server.lan.as_mut().expect("resolved guest");
                    host.hub.guests.update(index, |guest| {
                        apply(
                            slot.stack_mut(
                                &mut guest.inventory,
                                &mut guest.armor,
                                &mut guest.offhand,
                            ),
                            patch,
                        );
                    });
                } else {
                    self.journal_player(player);
                    apply(
                        slot.stack_mut(
                            &mut self.common.inventory,
                            &mut self.common.armor,
                            &mut self.common.offhand,
                        ),
                        patch,
                    );
                }
            }
            ItemPlace::PlayerCursor { menu } => {
                let guest = self
                    .native_menu_guest(menu)
                    .filter(|guest| !Self::menu_cursor_retired(guest))
                    .ok_or(ItemPlaceEditError::NotHeld)?;
                let player = guest.uuid;
                let (parked, carried) = if self.server.lan_acting == Some(guest.conn) {
                    (None, &self.common.carried)
                } else {
                    (
                        Some(
                            self.guest_index(guest.conn)
                                .ok_or(ItemPlaceEditError::NotHeld)?,
                        ),
                        &guest.carried,
                    )
                };
                let held = carried
                    .as_ref()
                    .filter(|stack| stack.count > 0 && stack.item.as_ref() != "minecraft:air")
                    .ok_or(ItemPlaceEditError::NotHeld)?;
                let Some((patch, _)) = prepare(EquipmentRead::resident(held), world_items!(self))?
                else {
                    return Ok(());
                };
                if let Some(index) = parked {
                    self.journal_parked_player(index);
                    let host = self.server.lan.as_mut().expect("resolved guest");
                    host.hub.guests.update(index, |guest| {
                        apply(
                            guest.carried.as_mut().expect("prepared cursor stack"),
                            patch,
                        );
                    });
                } else {
                    self.journal_player(player);
                    apply(
                        self.common.carried.as_mut().expect("prepared cursor stack"),
                        patch,
                    );
                }
            }
            _ => return Err(ItemPlaceEditError::NotHeld),
        }
        self.notice_menu_place_write(place);
        Ok(())
    }

    /// Runs the vanilla `ItemStack.onCraftedBy` effects for a stack a mod-delivered menu result
    /// owns. The stack mutation is journaled through the caller's transaction and the statistic
    /// is deferred until its root commit.
    pub(in crate::game) fn game_on_crafted_by(
        &mut self,
        place: &ItemPlace,
        tx: &TransactionContext,
        amount: i32,
    ) -> Result<(), CraftedByError> {
        if !self.transaction_context_is_current(tx) {
            return Err(CraftedByError::InvalidContext);
        }
        let (menu, slot) = match *place {
            ItemPlace::MenuSlot { menu, slot } => (menu, Some(slot)),
            ItemPlace::PlayerCursor { menu } => (menu, None),
            _ => return Err(CraftedByError::NotHeld),
        };
        let player = menu.fields().player;
        let handler_stack = if slot.is_some_and(|slot| {
            matches!(
                self.menu_slot_backing(menu, slot),
                Some(MenuSlotBacking::Place(_))
            )
        }) {
            // The handler owns this stack, not the slot's display mirror.
            Some(
                place
                    .with_stack(self, |stack| stack.project())
                    .flatten()
                    .ok_or(CraftedByError::NotHeld)?,
            )
        } else {
            None
        };
        let mut held = if let Some(stack) = &handler_stack {
            sunrise_gameplay_common::game::api_values::stack_value(stack)
                .map_err(|_| CraftedByError::Unavailable)?
        } else {
            // Read an engine stack directly when the menu owns the slot.
            match slot {
                Some(slot) => self.menu_slot(menu, slot),
                None => self.menu_carried(menu),
            }
            .flatten()
            .filter(|stack| stack.count > 0 && stack.item.as_ref() != "minecraft:air")
            .ok_or(CraftedByError::NotHeld)?
        };
        let item = held.item.clone();
        if held.item.as_ref() == "minecraft:filled_map"
            && held
                .data()
                .is_some_and(|data| data.map_post_processing.is_some())
        {
            self.storage_transaction_settled(|game, tx| {
                let maps = game.level_maps([&held]);
                Menu::map_crafted(Some(&maps), &mut held);
                // Reserve before calling the handler, which can make other maps. A refusal
                // aborts the reservation along with any writes made during the replacement.
                game.register_made_maps(maps);
                let written = match (handler_stack, slot) {
                    (Some(mut stack), _) => {
                        use sunrise_gameplay_common::game::api_values::TryIntoApi;
                        let processed = held
                            .try_into_api()
                            .map_err(|_| CraftedByError::Unavailable)?;
                        // Only these two components change. Keep the rest of the handler's patch.
                        stack.copy_component_from(&processed, "minecraft:map_id");
                        stack.copy_component_from(&processed, "minecraft:map_post_processing");
                        place
                            .exchange(game, tx, &mod_api::ItemResource::of(&stack), stack.count())
                            .map_err(|error| match error {
                                mod_api::TransactionError::CommitRefused => {
                                    CraftedByError::CommitRefused
                                }
                                mod_api::TransactionError::InvalidContext => {
                                    CraftedByError::InvalidContext
                                }
                                mod_api::TransactionError::Aborted(_)
                                | mod_api::TransactionError::Unavailable => {
                                    CraftedByError::Unavailable
                                }
                            })?
                            == stack.count()
                    }
                    (None, Some(slot)) => game.replace_crafted_menu_slot(menu, slot, held),
                    (None, None) => game.native_as_player_handle(player, |game| {
                        game.journal_player(player.uuid());
                        game.carried = Some(held);
                    }),
                };
                if !written {
                    return Err(CraftedByError::Unavailable);
                }
                game.notice_menu_place_write(place);
                Ok(())
            })
            .map_err(|error| match error {
                mod_api::TransactionError::Aborted(error) => error,
                mod_api::TransactionError::CommitRefused => CraftedByError::CommitRefused,
                mod_api::TransactionError::InvalidContext => CraftedByError::InvalidContext,
                mod_api::TransactionError::Unavailable => CraftedByError::Unavailable,
            })?;
        }
        self.defer_effect(DeferredEffect::ItemStat {
            player: player.uuid(),
            stat: "crafted",
            item,
            amount,
        });
        Ok(())
    }
    /// `ItemAccess.insert` at a player's place, an item entity's or an entity's slot: up to
    /// `stack.count` of `stack`, into the place where it's empty or holds the same item and
    /// components, up to the item's stack size. The rest goes where vanilla puts an item split
    /// off it: a player's into their inventory, else dropped as `Player.drop(stack, false)` at
    /// the root commit (a creative player's hand takes anything, and gives them one if they have
    /// none, as `ItemUtils.createFilledResult`); an item entity's dropped beside it at the root
    /// commit; an entity slot's into the entity's other slots (`forHandlerIndex`). How much
    /// went: 0 where `game_stack` reads nothing for want of the place. In transaction
    /// `tx`, which keeps each place as it was. The player's screen follows with the tick's sync.
    pub(in crate::game) fn game_insert(
        &mut self,
        place: &ItemPlace,
        resource: &ItemStack,
        amount: u32,
        tx: &TransactionContext,
    ) -> u32 {
        if !self.transaction_context_is_current(tx) {
            return 0;
        }
        let stack = ItemStack {
            count: amount,
            ..resource.clone()
        };
        if !self.places_simulated() || stack.count == 0 {
            return 0;
        }
        // A write to a backed mod menu's storage (#3385) is noticed; none is backed in vanilla.
        self.notice_menu_place_write(place);
        if let ItemPlace::Scratch(slot) = place {
            return self.scratch_insert(*slot, &stack);
        }
        let item_max = self.containers().max_stack(&stack);
        let max = match place {
            ItemPlace::MenuSlot { menu, slot } => self
                .menu_owned_slot_limit(*menu, *slot)
                .map_or(item_max, |limit| limit.min(item_max)),
            _ => item_max,
        };
        // Puts what of `rest` fits into `slot`.
        let fill = |slot: &mut Option<ItemStack>, rest: &mut ItemStack| {
            let held = slot.as_ref().map_or(0, |held| held.count);
            let room = match slot {
                Some(h) if held > 0 && !h.same_item_same_components(rest) => 0,
                _ => max.saturating_sub(held),
            };
            let put = room.min(rest.count);
            if put > 0 {
                *slot = Some(ItemStack {
                    count: held + put,
                    ..rest.clone()
                });
                rest.count -= put;
            }
        };
        let mut rest = stack.clone();
        if let ItemPlace::MenuSlot { menu, slot } = place {
            // The slot alone: what doesn't fit stays out.
            if self.native_menu_storage(*menu, *slot).is_none() {
                return 0;
            }
            self.journal_menus(menu.fields().player);
            if let Some(held) = self.native_menu_storage(*menu, *slot) {
                fill(held, &mut rest);
            }
            return stack.count - rest.count;
        }
        if let ItemPlace::ItemEntity { id } = place {
            let Some(e) = self.common.dropped.iter().find(|e| e.id == *id) else {
                return 0;
            };
            let at = [e.position.x, e.position.y, e.position.z];
            let mut held = Some(e.stack.clone());
            fill(&mut held, &mut rest);
            if let Some(held) = held.filter(|_| rest.count < stack.count) {
                self.journal_set_item_entity(*id, held);
            }
            if rest.count > 0 {
                self.after_commit(After::Spawn(at, rest));
            }
            return stack.count;
        }
        if let ItemPlace::EntitySlot { entity, slot } = place {
            if !self
                .edit_entity_slots(*entity, |slots, _| *slot < slots.len())
                .unwrap_or(false)
            {
                return 0;
            }
            self.journal_entity(*entity);
            return self
                .edit_entity_slots(*entity, |slots, _| {
                    fill(&mut slots[*slot], &mut rest);
                    for (i, held) in slots.iter_mut().enumerate() {
                        if i != *slot {
                            fill(held, &mut rest);
                        }
                    }
                    stack.count - rest.count
                })
                .unwrap_or(0);
        }
        if let ItemPlace::Equipment { entity, slot } = place {
            // One slot only: what doesn't fit stays out (the handler's
            // capacity already refused what the kind cannot wear).
            let Some(mut held) = self.equipment_stack(*entity, *slot) else {
                return 0;
            };
            fill(&mut held, &mut rest);
            if rest.count == stack.count {
                return 0;
            }
            let written = held.filter(|h| h.count > 0);
            // The journal keeps what was, only for a write that really lands:
            // a refused one (a client's copy, no epoch) moved nothing.
            self.journal_equipment(*entity);
            if !self.write_entity_equipment(*entity, *slot, written) {
                return 0;
            }
            return stack.count - rest.count;
        }
        let Some(player) = self.place_player(place) else {
            return 0;
        };
        let mut went = 0;
        self.native_as_player(player, |g| {
            if g.player_place(place).is_none() {
                return;
            }
            went = stack.count;
            g.journal_player(player);
            if g.creative && matches!(place, ItemPlace::PlayerHand { .. }) {
                let have = g.inventory.iter().flatten();
                if !have.clone().any(|h| h.same_item_same_components(&stack)) {
                    g.add_stack_to_inventory(ItemStack { count: 1, ..stack });
                }
                return;
            }
            let mut written = g.player_place(place).cloned().flatten();
            fill(&mut written, &mut rest);
            g.write_player_place(place, written);
            if let Some(rest) = (rest.count > 0)
                .then(|| g.add_stack_to_inventory(rest))
                .flatten()
            {
                // What is left drops at the player, unless they are in another level than
                // this one: then it stays the caller's.
                if g.acting_in_level_in_place() {
                    g.drop_after_commit(player, rest);
                } else {
                    went -= rest.count;
                }
            }
        });
        went
    }
    /// `ItemAccess.extract` at those places: up to `amount` of the item and components of
    /// `resource`, how many came out. An item entity emptied stays, empty, until the root commit
    /// removes it. A creative player's hand gives without losing any (`forInfiniteMaterials`).
    pub(in crate::game) fn game_extract(
        &mut self,
        place: &ItemPlace,
        resource: &ItemStack,
        amount: u32,
        tx: &TransactionContext,
    ) -> u32 {
        if !self.transaction_context_is_current(tx) {
            return 0;
        }
        if !self.places_simulated() {
            return 0;
        }
        self.notice_menu_place_write(place);
        if let ItemPlace::Scratch(slot) = place {
            return self.common.scratch_extract(*slot, resource, amount);
        }
        let items = self.common.res.shared_items();
        let taken = |held: Option<&ItemStack>| {
            held.filter(|held| held.same_item_same_components_with_prototype(resource, &items))
                .map_or(0, |held| held.count.min(amount))
        };
        let left = |held: &ItemStack, took: u32| ItemStack {
            count: held.count - took,
            ..held.clone()
        };
        if let ItemPlace::MenuSlot { menu, slot } = place {
            let Some(held) = self.native_menu_storage(*menu, *slot) else {
                return 0;
            };
            let took = taken(held.as_ref());
            if took == 0 {
                return 0;
            }
            self.journal_menus(menu.fields().player);
            if let Some(held) = self.native_menu_storage(*menu, *slot) {
                *held = held.as_ref().map(|h| left(h, took)).filter(|h| h.count > 0);
            }
            return took;
        }
        if let ItemPlace::ItemEntity { id } = place {
            let Some(e) = self.common.dropped.iter().find(|e| e.id == *id) else {
                return 0;
            };
            let took = taken(Some(&e.stack));
            let kept = left(&e.stack, took);
            if took > 0 {
                self.journal_set_item_entity(*id, kept);
            }
            return took;
        }
        if let ItemPlace::EntitySlot { entity, slot } = place {
            let took = self
                .edit_entity_slots(*entity, |slots, _| {
                    taken(slots.get(*slot).and_then(Option::as_ref))
                })
                .unwrap_or(0);
            if took == 0 {
                return 0;
            }
            self.journal_entity(*entity);
            return self
                .edit_entity_slots(*entity, |slots, _| {
                    let kept = slots[*slot].as_ref().map(|held| left(held, took));
                    slots[*slot] = kept.filter(|kept| kept.count > 0);
                    took
                })
                .unwrap_or(0);
        }
        if let ItemPlace::Equipment { entity, slot } = place {
            let Some(held) = self.equipment_stack(*entity, *slot) else {
                return 0;
            };
            let took = taken(held.as_ref());
            if took == 0 {
                return 0;
            }
            let kept = held
                .as_ref()
                .map(|held| left(held, took))
                .filter(|kept| kept.count > 0);
            self.journal_equipment(*entity);
            if !self.write_entity_equipment(*entity, *slot, kept) {
                return 0;
            }
            return took;
        }
        let Some(player) = self.place_player(place) else {
            return 0;
        };
        let mut took = 0;
        self.native_as_player(player, |g| {
            let creative = g.creative && matches!(place, ItemPlace::PlayerHand { .. });
            let Some(slot) = g.player_place(place) else {
                return;
            };
            took = taken(slot.as_ref());
            if took == 0 || creative {
                return;
            }
            g.journal_player(player);
            let kept = g
                .player_place(place)
                .and_then(|slot| slot.as_ref().map(|held| left(held, took)));
            g.write_player_place(place, kept.filter(|kept| kept.count > 0));
        });
        took
    }
    /// Project the equipment before a write. Outer `None` is an unavailable place; inner
    /// `None` is an empty slot that can receive an item.
    pub(in crate::game::server_side) fn equipment_stack(
        &self,
        entity: u32,
        slot: EquipmentSlot,
    ) -> Option<Option<ItemStack>> {
        self.equipment_stack_source(entity, slot)
            .map(|stack| stack.map(|stack| stack.project().into_owned()))
    }

    /// A mob's equipment with its worn overlays, or an armor stand's resident equipment.
    pub(in crate::game::server_side) fn equipment_stack_source(
        &self,
        entity: u32,
        slot: EquipmentSlot,
    ) -> Option<Option<EquipmentRead<'_>>> {
        if let Some(mob) = self
            .common
            .mobs
            .iter()
            .find(|m| m.id == entity && !m.removed)
        {
            return Some(finalize::equipment_slot(mob, slot));
        }
        let stand = self.common.armor_stands.iter().find(|a| a.id == entity)?;
        Some(
            stand
                .equipment
                .get(slot as usize)
                .map(EquipmentRead::resident),
        )
    }
    /// The equipment write `set_equipment` makes (#932), deferred: whether it
    /// landed. False where the entity is gone or a client's copy refuses.
    pub(in crate::game::server_side) fn write_entity_equipment(
        &mut self,
        entity: u32,
        slot: EquipmentSlot,
        stack: Option<ItemStack>,
    ) -> bool {
        let Some(epoch) = self.common.native_lifecycle.epoch() else {
            return false;
        };
        let living = LivingRef::new(mod_api::EntityRef::new(epoch, entity));
        self.write_living_equipment(living, slot, stack, true)
    }
    /// Drops `stack` at the middle of the block at `pos`, as a block entity's handler gives up
    /// what it can't put back.
    pub(in crate::game) fn drop_at(&mut self, pos: [i32; 3], stack: ItemStack) {
        if self.entity_ids_exhausted() {
            return;
        }
        if self.places_simulated() && stack.count > 0 {
            // `Block.popResource`'s default pickup delay, at the root commit.
            self.after_commit(After::Drop(pos, stack));
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use sunrise_gameplay_common::game::TryIntoApi;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
    use sunrise_gameplay_common::game::native_menu_refs_common::NativeMenuRefsCommonGameExt1 as _;
    use sunrise_gameplay_common::game::server_side::native_item_places::tests::guest;

    use super::super::BlockEntity;
    use mod_api::HostTransaction;
    use mod_api::ServerHost;
    use mod_api::{ENTITY_ITEMS, ItemPlace};
    use mod_api::{ItemResource, MenuRef, TransactionError};
    use std::sync::Arc;
    use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
    use sunrise_gameplay_common::game::tables::GameItems as _;
    use sunrise_items::items::ItemStack;
    use sunrise_sim::lan::{NetConfig, hub::HubConfig};

    /// A world open to LAN on a dedicated server, as a guest's actions run.
    pub fn open_to_lan(game: &mut crate::game::ServerGame<'_>) {
        game.options.dedicated = Some(Vec::new());
        let settings = super::super::lan::LanSettings {
            port: 0,
            guest_commands: false,
            force_game_mode: false,
        };
        let net = NetConfig {
            announce: false,
            ..NetConfig::lan(0, "PlaceTest".into(), 2)
        };
        let config = HubConfig {
            default_game_mode: 0,
            guest_commands: false,
            force_game_mode: false,
            hardcore: false,
            view_distance: 10,
            simulation_distance: 10,
            dimension: "minecraft:overworld".into(),
            allow_flight: false,
            seed: 0,
            max_players: 2,
            integrated: false,
            levels: Default::default(),
            level_seeds: Default::default(),
        };
        game.lan_host(settings, net, config).unwrap();
    }

    /// A zombie mob, standing at the origin.
    pub fn zombie(game: &mut crate::game::ServerGame<'_>) -> u32 {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        game.mobs.push(sunrise_sim::mobs::Mob::new(
            30,
            kind,
            sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5),
            0.0,
        ));
        30
    }

    pub fn saved_mob_attribute(
        game: &crate::game::ServerGame<'_>,
        entity: u32,
        attribute: &str,
    ) -> mod_api::Tag {
        use mod_api::Tag;
        let mob = game.mobs.iter().find(|mob| mob.id == entity).unwrap();
        let saved = sunrise_sim::entity_nbt::mob_to_nbt(
            mob,
            [0, 0, 0, 1],
            None,
            None,
            |_| None,
            game.items(),
        );
        let Some(Tag::List(attributes)) = saved.get("attributes") else {
            panic!("the mob must save its live attributes");
        };
        let id = Tag::String(attribute.into());
        attributes
            .iter()
            .find(|entry| entry.get("id") == Some(&id))
            .unwrap()
            .clone()
    }

    /// The open menu of guest 1 (UUID 123).
    pub fn menu_of(game: &crate::game::ServerGame<'_>) -> MenuRef {
        let player = game.player(123).unwrap();
        game.player_container_menu(player).unwrap()
    }

    /// One write at `place` in a transaction of its own: how many went in (`amount > 0`) or
    /// came out (`amount < 0`), committed or aborted.
    pub fn write(
        game: &mut crate::game::ServerGame<'_>,
        place: &ItemPlace,
        item: &str,
        amount: i32,
        commit: bool,
    ) -> u32 {
        let resource = ItemResource::of(&ItemStack::new(item, 1).try_into_api().unwrap());
        let host: &mut dyn ServerHost = game;
        let mut moved = 0;
        let _ = host.transaction(|host, tx| {
            moved = match amount > 0 {
                true => place
                    .insert(host, tx, &resource, amount.unsigned_abs())
                    .unwrap(),
                false => place
                    .extract(host, tx, &resource, amount.unsigned_abs())
                    .unwrap(),
            };
            if commit { Ok(()) } else { Err(()) }
        });
        moved
    }

    /// The guests' inventories, by connection.
    pub fn inventories(game: &crate::game::ServerGame<'_>) -> Vec<Vec<Option<ItemStack>>> {
        let guests = &game.server.lan.as_ref().unwrap().hub.guests;
        guests.iter().map(|g| g.inventory.clone()).collect()
    }

    pub type Offer = (usize, ItemStack);

    #[derive(Debug, PartialEq, Eq)]
    pub struct TradeRefused;

    /// Two guests' trade on a menu, as the docs' recipe has it: each offer's index must still
    /// hold exactly what was offered, and goes whole to the other player, or nothing moves.
    pub fn trade(
        game: &mut crate::game::ServerGame<'_>,
        a: &[Offer],
        b: &[Offer],
    ) -> Result<(), TradeRefused> {
        let host: &mut dyn ServerHost = game;
        let items = |host: &mut dyn ServerHost, id| {
            let entity = host.entity(id).unwrap();
            entity.capability(host, &ENTITY_ITEMS, ()).unwrap()
        };
        let (seller, buyer) = (items(host, 20), items(host, 21));
        host.transaction(|host, tx| {
            for (from, to, offer) in [(&seller, &buyer, a), (&buyer, &seller, b)] {
                for (index, stack) in offer {
                    let offered = ItemResource::of(&stack.try_into_api().unwrap());
                    let still = from.resource(host, *index).as_ref() == Some(&offered)
                        && from.amount(host, *index) == stack.count;
                    if !still
                        || from.extract_at(host, tx, *index, &offered, stack.count)
                            != Ok(stack.count)
                        || to.insert(host, tx, &offered, stack.count) != Ok(stack.count)
                    {
                        return Err(());
                    }
                }
            }
            Ok(())
        })
        .map_err(|err| match err {
            TransactionError::Aborted(()) => TradeRefused,
            other => panic!("unexpected transaction error: {other:?}"),
        })
    }

    /// A seller with 3 diamonds in slot 0 and a buyer with 5 emeralds in slot 0, beside
    /// `buyer_rest` in every other slot and the off hand.
    pub fn traders(game: &mut crate::game::ServerGame<'_>, buyer_rest: Option<ItemStack>) {
        open_to_lan(game);
        let mut seller = guest(1);
        seller.inventory[0] = Some(ItemStack::new("minecraft:diamond", 3));
        let mut buyer = guest(2);
        buyer.inventory[0] = Some(ItemStack::new("minecraft:emerald", 5));
        for slot in &mut buyer.inventory[1..36] {
            *slot = buyer_rest.clone();
        }
        buyer.offhand = buyer_rest;
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .extend([seller, buyer]);
    }

    /// A hopper at (0, 64, 0) facing `facing` holding `stack`, in an otherwise empty world.
    pub fn hopper_with(
        game: &mut crate::game::ServerGame<'_>,
        facing: &str,
        stack: Option<ItemStack>,
    ) {
        use sunrise_level::chunk::Chunk;
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let base = game
            .pack
            .assets
            .registry
            .default_state("minecraft:hopper")
            .unwrap();
        let state = game.with_props(base, &[("facing", facing)]).unwrap();
        game.world.set(0, 64, 0, state).unwrap();
        let mut slots = vec![None; 5];
        slots[0] = stack;
        game.world
            .block_entities
            .insert((0, 64, 0), BlockEntity::Chest(slots));
    }

    pub fn pig(game: &mut crate::game::ServerGame<'_>, id: u32, y: f64) {
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        game.mobs.push(sunrise_sim::mobs::Mob::new(
            id,
            kind,
            sunrise_core::geometry::Vec3::new(0.5, y, 0.5),
            0.0,
        ));
    }

    pub fn hopper_slot(game: &crate::game::ServerGame<'_>) -> Option<ItemStack> {
        match game.world.block_entities.get(&(0, 64, 0)) {
            Some(BlockEntity::Chest(slots)) => slots[0].clone(),
            _ => panic!("the hopper is gone"),
        }
    }

    /// A chunk at the origin, open to LAN, with a full water cauldron at (2, 64, 0).
    pub fn with_cauldron(game: &mut crate::game::ServerGame<'_>) -> mod_api::LevelRef {
        use mod_api::BlockUpdateFlags;
        use sunrise_level::chunk::Chunk;
        open_to_lan(game);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let level = game.level("minecraft:overworld").unwrap();
        let state = sunrise_world::chunk::BlockState {
            name: "minecraft:water_cauldron".to_owned(),
            properties: [("level".to_owned(), "3".to_owned())].into_iter().collect(),
        };
        let full = game.pack.assets.registry.resolve(&state).unwrap();
        let flags = BlockUpdateFlags::NEIGHBORS;
        assert_eq!(
            game.level_set_block(level, [2, 64, 0], full, flags),
            Ok(true)
        );
        level
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_menu_refs_common::NativeMenuRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::transactions::scratch::ScratchGameExt1 as _;
