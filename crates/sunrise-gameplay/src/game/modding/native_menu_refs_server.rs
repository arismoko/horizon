use super::lan_crafting::machine_menu_kind;
use mod_api::__private::game::*;
use mod_api::{
    EntityRef, HostTransaction, ItemResource, LevelReadError, MenuRef, MenuSlotBacking, PlayerRef,
    ResourceHandler,
};
use sunrise_gameplay_common::game::api_values::stack_value;
use sunrise_gameplay_common::game::equip_answers::EquipAnswersGameExt1 as _;
use sunrise_items::items::ItemStack;
use sunrise_level::containers::BlockEntity;
use sunrise_sim::{
    lan::{
        guest::Guest,
        menu::{SlotBacking, Slots, VEHICLE_KEY_Y},
    },
    menu::{Env, Kind, LiveSlotRules},
};

impl crate::game::ServerGame<'_> {
    /// `ServerPlayer.closeContainer` of a guest's menu: queue the client's close before
    /// running normal server teardown.
    pub(in crate::game) fn menu_close_container(&mut self, menu: MenuRef) -> bool {
        let window = menu.fields().window;
        if !self.common.is_logical_server() || window == 0 {
            return false;
        }
        let Some(guest) = self.native_menu_guest(menu) else {
            return false;
        };
        if guest.disconnect_pending || guest.menu.as_ref().is_some_and(|menu| menu.removed) {
            return false;
        }
        self.request_menu_close(guest.conn, window, guest.menu_generation);
        true
    }

    /// `DataSlot.set` of an owning mod's blockless menu, journaled by window generation.
    pub(in crate::game) fn menu_set_data_slot(
        &mut self,
        menu: MenuRef,
        index: usize,
        value: i32,
    ) -> Result<bool, mod_api::MenuDataRefusal> {
        use mod_api::MenuDataRefusal::*;
        if !self.common.is_logical_server() {
            return Err(Unsupported);
        }
        let guest = self.native_menu_guest(menu).ok_or(Stale)?;
        let open = guest
            .menu
            .as_ref()
            .filter(|m| m.slots == Slots::Native && m.mod_type.is_some() && !m.removed)
            .ok_or(Unsupported)?;
        let &committed = open.blockless_data.get(index).ok_or(InvalidIndex)?;
        let conn = guest.conn;
        let key = (conn, guest.menu_generation, index);
        if self.pending_menu_data(key).unwrap_or(committed) == value {
            return Ok(false);
        }
        if !self.journal_menu_data(key, value) {
            self.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(conn, |guest| {
                    guest.set_menu_data(index, value);
                });
        }
        Ok(true)
    }

    pub fn native_menu_guest(&self, menu: MenuRef) -> Option<&Guest> {
        let guest = self.native_menu_player(menu.fields().player)?;
        let host = self.server.lan.as_ref()?;
        (host
            .hub
            .holder_guest(guest.uuid, self.common.dim.id())?
            .conn
            == guest.conn)
            .then_some(())?;
        (guest.menu_generation == menu.fields().generation).then_some(())?;
        if let Some(open) = &guest.menu {
            (open.window == menu.fields().window).then_some(guest)
        } else {
            let play = host.play.get(&guest.conn);
            let menus = host.menus.get(&guest.conn);
            if menu.fields().window != 0
                || menus.is_some_and(|m| {
                    m.closing
                        || m.crafting.table.is_some()
                        || m.station.is_some()
                        || m.merchant.is_some()
                })
                || play.is_some_and(|p| p.lectern.is_some() || p.mount_window.is_some())
            {
                return None;
            }
            Some(guest)
        }
    }

    pub(in crate::game) fn server_player_container_menu(
        &self,
        player: PlayerRef,
    ) -> Option<MenuRef> {
        self.player_read_both(player)?;
        let guest = self.native_menu_player(player)?;
        let menu = mod_api::__private::game::menu_ref(
            player,
            guest.menu.as_ref().map_or(0, |m| m.window),
            guest.menu_generation,
        );
        self.native_menu_guest(menu)?;
        Some(menu)
    }

    /// Container id (0 for the player's inventory menu).
    pub(in crate::game) fn menu_container_id(&self, menu: MenuRef) -> Option<i32> {
        self.native_menu_guest(menu)?;
        Some(menu.fields().window)
    }

    pub(in crate::game) fn server_menu_kind(&self, menu: MenuRef) -> Option<Option<String>> {
        let guest = self.native_menu_guest(menu)?;
        match &guest.menu {
            Some(m) => Some(Some(
                self.server
                    .lan
                    .as_ref()?
                    .hub
                    .registry_name("minecraft:menu", m.kind)?
                    .to_owned(),
            )),
            None => Some(None),
        }
    }

    pub(in crate::game) fn server_menu_slot_count(&self, menu: MenuRef) -> Option<usize> {
        let guest = self.native_menu_guest(menu)?;
        Some(
            guest
                .menu
                .as_ref()
                .map_or(46, sunrise_sim::lan::menu::Menu::slot_count),
        )
    }

    /// `AbstractContainerMenu.getStateId` on the server.
    pub(in crate::game) fn menu_state_id(&self, menu: MenuRef) -> Option<i32> {
        let guest = self.native_menu_guest(menu)?;
        Some(
            guest
                .menu
                .as_ref()
                .map_or(guest.menu_state, |open| open.state),
        )
    }

    /// Carried (cursor) stack; outer None means unavailable.
    pub(in crate::game) fn server_menu_carried(&self, menu: MenuRef) -> Option<Option<ItemStack>> {
        let guest = self.native_menu_guest(menu)?;
        if Self::menu_cursor_retired(guest) {
            return Some(None);
        }
        Some(if self.server.lan_acting == Some(guest.conn) {
            self.common.carried.clone()
        } else {
            guest.carried.clone()
        })
    }

    /// Whether the guest's open menu has been removed: its cursor went back to the inventory's
    /// own, which the closing handle must not read or edit.
    pub(in crate::game) fn menu_cursor_retired(guest: &Guest) -> bool {
        guest.menu.as_ref().is_some_and(|open| open.removed)
    }

    /// Full stack at a menu index. Outer None means unavailable or out of
    /// range; Some(None) means an empty slot.
    pub(in crate::game) fn server_menu_slot(
        &self,
        menu: MenuRef,
        slot: usize,
    ) -> Option<Option<ItemStack>> {
        let guest = self.native_menu_guest(menu)?;
        let backing = self.menu_slot_backing_for(guest, slot)?;
        let acting = self.server.lan_acting == Some(guest.conn);
        Some(match backing {
            MenuSlotBacking::BlockEntity { .. } | MenuSlotBacking::Entity { .. } => {
                let world = self.native_level_world(self.level(&guest.dimension)?)?;
                guest.menu.as_ref()?.slots.get(world, guest, slot)
            }
            MenuSlotBacking::Player(n @ 0..36) => {
                if acting {
                    self.common.inventory.get(n)?.clone()
                } else {
                    guest.inventory.get(n)?.clone()
                }
            }
            MenuSlotBacking::Player(n @ 36..40) => {
                if acting {
                    self.common.armor[39 - n].clone()
                } else {
                    guest.armor[39 - n].clone()
                }
            }
            MenuSlotBacking::Player(_) => {
                if acting {
                    self.common.offhand.clone()
                } else {
                    guest.offhand.clone()
                }
            }
            MenuSlotBacking::CraftGrid(n) => guest.craft_grid.get(n)?.clone(),
            MenuSlotBacking::CraftResult if guest.menu.is_some() => {
                self.with_live_slot_rules(guest, backing, slot, |rules, env, _| {
                    rules.held_container_slot(env, slot).cloned()
                })?
            }
            MenuSlotBacking::CraftResult => guest.craft_result.clone(),
            MenuSlotBacking::Ender(n) => {
                if acting {
                    self.server.ender_items.get(n)?.clone()
                } else {
                    guest.ender_items.get(n)?.clone()
                }
            }
            MenuSlotBacking::Native(n) | MenuSlotBacking::Owned(n) => {
                guest.native_menu.get(n)?.clone()
            }
            // The place's mirror, refreshed after each click and each tick.
            MenuSlotBacking::Place(_) => guest.native_menu.get(slot)?.clone(),
            _ => unreachable!("MenuSlotBacking: unhandled variant"),
        })
    }

    /// The storage menu slot `slot` shows (`Slot.container` and `getContainerSlot`).
    pub fn menu_slot_backing(&self, menu: MenuRef, slot: usize) -> Option<MenuSlotBacking> {
        let guest = self.native_menu_guest(menu)?;
        self.menu_slot_backing_for(guest, slot)
    }

    pub(in crate::game) fn menu_slot_backing_for(
        &self,
        guest: &Guest,
        slot: usize,
    ) -> Option<MenuSlotBacking> {
        let Some(open) = &guest.menu else {
            return match slot {
                0 => Some(MenuSlotBacking::CraftResult),
                1..=4 => Some(MenuSlotBacking::CraftGrid(slot - 1)),
                5..=8 => Some(MenuSlotBacking::Player(39 - (slot - 5))),
                9..=35 => Some(MenuSlotBacking::Player(slot)),
                36..=44 => Some(MenuSlotBacking::Player(slot - 36)),
                45 => Some(MenuSlotBacking::Player(40)),
                _ => None,
            };
        };
        if slot >= open.slot_count() {
            return None;
        }
        let level = self.level(&guest.dimension)?;
        let world = self.native_level_world(level)?;
        if open
            .slots
            .block_slots(world)
            .is_some_and(|n| n != open.size)
        {
            return None;
        }
        if slot >= open.size {
            return match slot - open.size {
                n @ 0..=26 => Some(MenuSlotBacking::Player(n + 9)),
                n @ 27..=35 => Some(MenuSlotBacking::Player(n - 27)),
                _ => None,
            };
        }
        if open
            .mod_type
            .and_then(|kind| kind.crafting)
            .is_some_and(|craft| slot == craft.result)
        {
            return Some(MenuSlotBacking::CraftResult);
        }
        if let Some(linked) = open
            .linked_slots(world)
            .and_then(|links| links.backing(slot))
        {
            return linked.map(|(pos, n)| {
                if pos.1 == VEHICLE_KEY_Y {
                    MenuSlotBacking::Entity {
                        entity: EntityRef::new(level.epoch(), pos.2 as u32),
                        slot: n,
                    }
                } else {
                    MenuSlotBacking::BlockEntity {
                        level,
                        pos: [pos.0, pos.1, pos.2],
                        slot: n,
                    }
                }
            });
        }
        Some(match open.slots.backing(world, slot)? {
            SlotBacking::Block(pos, n) if pos.1 == VEHICLE_KEY_Y => MenuSlotBacking::Entity {
                entity: EntityRef::new(level.epoch(), pos.2 as u32),
                slot: n,
            },
            SlotBacking::Block(pos, n) => MenuSlotBacking::BlockEntity {
                level,
                pos: [pos.0, pos.1, pos.2],
                slot: n,
            },
            SlotBacking::Ender(n) => MenuSlotBacking::Ender(n),
            SlotBacking::Native(n) => match self.native_menu_place(guest.conn, open.window, n) {
                Some(place) => MenuSlotBacking::Place(place),
                None if open.owned_slots().is_some() => MenuSlotBacking::Owned(n),
                None => MenuSlotBacking::Native(n),
            },
        })
    }

    fn is_unbound_linked_slot(&self, guest: &Guest, slot: usize) -> bool {
        let Some(open) = &guest.menu else {
            return false;
        };
        if slot >= open.slot_count() {
            return false;
        }
        let Some(world) = self
            .level(&guest.dimension)
            .and_then(|level| self.native_level_world(level))
        else {
            return false;
        };
        open.slots.block_slots(world).is_none_or(|n| n == open.size)
            && matches!(
                open.linked_slots(world)
                    .and_then(|links| links.backing(slot)),
                Some(None)
            )
    }

    pub(in crate::game) fn menu_slot_holds(
        &self,
        menu: MenuRef,
        slot: usize,
        stack: &ItemStack,
    ) -> Option<u32> {
        let guest = self.native_menu_guest(menu)?;
        if self.is_unbound_linked_slot(guest, slot) {
            return Some(0);
        }
        let backing = self.menu_slot_backing_for(guest, slot)?;
        if matches!(backing, MenuSlotBacking::Place(_))
            && !self.held_places_bound(guest.conn, menu.fields().player)
        {
            return None;
        }
        if !self.backed_menu_slot_enabled(guest, slot) {
            return Some(0);
        }
        if matches!(backing, MenuSlotBacking::Place(_)) {
            return None;
        }
        self.with_live_slot_rules(guest, backing, slot, |rules, env, _| {
            rules.slot_holds(env, slot, stack)
        })?
    }

    pub(in crate::game) fn menu_slot_gives(&self, menu: MenuRef, slot: usize) -> Option<bool> {
        let guest = self.native_menu_guest(menu)?;
        if self.is_unbound_linked_slot(guest, slot) {
            return Some(false);
        }
        let backing = self.menu_slot_backing_for(guest, slot)?;
        if matches!(backing, MenuSlotBacking::Place(_))
            && !self.held_places_bound(guest.conn, menu.fields().player)
        {
            return None;
        }
        if !self.backed_menu_slot_enabled(guest, slot) {
            return Some(false);
        }
        if matches!(backing, MenuSlotBacking::Place(_)) {
            return None;
        }
        self.with_live_slot_rules(guest, backing, slot, |rules, env, held| {
            rules.slot_gives(env, slot, held)
        })?
    }

    /// Server-action probe for [`Self::menu_slot_holds`]: asks a handler-backed slot's live
    /// [`ResourceHandler`] (`is_valid`, then `capacity_int` as in NeoForge's
    /// `ResourceHandlerSlot.getMaxStackSize`) and delegates every other slot to the shared
    /// read-only evaluator.
    pub(in crate::game) fn menu_probe_slot_holds(
        &mut self,
        menu: MenuRef,
        slot: usize,
        stack: &mod_api::ItemStack,
    ) -> Result<Option<u32>, LevelReadError> {
        let Some(guest) = self.native_menu_guest(menu) else {
            return Ok(None);
        };
        let conn = guest.conn;
        if let Some(MenuSlotBacking::Place(place)) = self.menu_slot_backing_for(guest, slot) {
            if !self.held_places_bound(conn, menu.fields().player) {
                return Ok(None);
            }
            if !self.backed_menu_slot_enabled(guest, slot) {
                return Ok(Some(0));
            }
            let Some((handler, index)) = menu_slot_place_handler(place, self, menu.fields().player)
            else {
                return Ok(None);
            };
            let resource = ItemResource::of(stack);
            let handler_holds = if handler.is_valid(self, index, &resource) {
                u32::try_from(handler.capacity_int(self, index, Some(&resource))).unwrap_or(0)
            } else {
                0
            };
            // A click also applies the menu's own rules to the slot, so the probe does too.
            let rules = self.backed_slot_rules(conn, slot, &resource, stack.count());
            return Ok(Some(match rules {
                Some((true, _, max)) => handler_holds.min(max),
                _ => 0,
            }));
        }
        let Ok(stack) = stack_value(stack) else {
            return Ok(None);
        };
        Ok(self.menu_slot_holds(menu, slot, &stack))
    }

    /// Server-action probe for [`Self::menu_slot_gives`]: simulates taking one item from a
    /// handler-backed slot's live [`ResourceHandler`] (NeoForge's
    /// `ResourceHandlerSlot.mayPickup`) and delegates every other slot to the shared read-only
    /// evaluator.
    pub(in crate::game) fn menu_probe_slot_gives(
        &mut self,
        menu: MenuRef,
        slot: usize,
    ) -> Result<Option<bool>, LevelReadError> {
        let Some(guest) = self.native_menu_guest(menu) else {
            return Ok(None);
        };
        let conn = guest.conn;
        if let Some(MenuSlotBacking::Place(place)) = self.menu_slot_backing_for(guest, slot) {
            if !self.held_places_bound(conn, menu.fields().player) {
                return Ok(None);
            }
            if !self.backed_menu_slot_enabled(guest, slot) {
                return Ok(Some(false));
            }
            let Some((handler, index)) = menu_slot_place_handler(place, self, menu.fields().player)
            else {
                return Ok(None);
            };
            let Some(resource) = handler.resource(self, index) else {
                return Ok(Some(false));
            };
            // NeoForge ResourceHandlerSlot.mayPickup: simulate extracting one, without
            // publishing any writes or change notices from the handler.
            let result: Result<(), mod_api::TransactionError<bool>> =
                self.transaction(|game, tx| {
                    Err(handler.extract_at(game, tx, index, &resource, 1) == Ok(1))
                });
            let amount = handler.amount(self, index);
            let rules = self.backed_slot_rules(conn, slot, &resource, amount);
            return Ok(match result {
                Err(mod_api::TransactionError::Aborted(ok)) => {
                    Some(ok && rules.is_some_and(|(_, pickup, _)| pickup))
                }
                _ => None,
            });
        }
        Ok(self.menu_slot_gives(menu, slot))
    }

    /// The menu's own rules for backed slot `slot` ([`Self::place_menu_rules`]), the same answer a
    /// click on it is admitted by. None when the menu is no longer backed.
    fn backed_slot_rules(
        &self,
        conn: u32,
        slot: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> Option<(bool, bool, u32)> {
        let (_, _, locked) = self.menu_places_of(conn)?;
        let size = self
            .server
            .lan
            .as_ref()?
            .hub
            .guest(conn)?
            .menu
            .as_ref()?
            .size;
        Some(self.place_menu_rules(conn, size, locked, slot, resource, amount))
    }

    pub(in crate::game) fn menu_slot_gives_with_stack(
        &self,
        menu: MenuRef,
        slot: usize,
        stack: &ItemStack,
    ) -> Option<bool> {
        let guest = self.native_menu_guest(menu)?;
        let backing = self.menu_slot_backing_for(guest, slot)?;
        self.with_live_slot_rules(guest, backing, slot, |rules, env, _| {
            rules.slot_gives(env, slot, Some(stack))
        })?
    }

    pub(in crate::game) fn menu_slot_holds_with_stack(
        &self,
        menu: MenuRef,
        slot: usize,
        stack: &ItemStack,
    ) -> Option<u32> {
        let guest = self.native_menu_guest(menu)?;
        let backing = self.menu_slot_backing_for(guest, slot)?;
        self.with_live_slot_rules(guest, backing, slot, |rules, env, _| {
            rules.slot_holds(env, slot, stack)
        })?
    }

    pub(in crate::game) fn menu_owned_slot_limit(&self, menu: MenuRef, slot: usize) -> Option<u32> {
        let open = self.native_menu_guest(menu)?.menu.as_ref()?;
        let limit = *open.owned_slots()?.get(slot)?;
        Some(u32::from(limit))
    }

    pub(in crate::game) fn replace_crafted_menu_slot(
        &mut self,
        menu: MenuRef,
        slot: usize,
        stack: ItemStack,
    ) -> bool {
        let player = menu.fields().player;
        match self.menu_slot_backing(menu, slot) {
            Some(MenuSlotBacking::Player(index)) => self.native_as_player_handle(player, |game| {
                game.journal_player(player.uuid());
                let held = match index {
                    0..36 => &mut game.inventory[index],
                    36..40 => &mut game.armor[39 - index],
                    _ => &mut game.offhand,
                };
                *held = Some(stack);
            }),
            Some(MenuSlotBacking::BlockEntity { .. } | MenuSlotBacking::Entity { .. }) => {
                let Some((pos, index)) = self.native_menu_guest(menu).and_then(|guest| {
                    guest
                        .menu
                        .as_ref()?
                        .slots
                        .held_block_slot(&self.common.world, guest, slot)
                }) else {
                    return false;
                };
                if pos.1 == VEHICLE_KEY_Y {
                    let entity = pos.2 as u32;
                    self.journal_entity(entity);
                    return self
                        .edit_entity_slots(entity, |slots, _| {
                            let Some(held) = slots.get_mut(index) else {
                                return false;
                            };
                            *held = Some(stack);
                            true
                        })
                        .unwrap_or(false);
                }
                self.journal_block_entity_slot(pos, index);
                let Some(held) = self
                    .common
                    .world
                    .block_entities
                    .get_mut(&pos)
                    .and_then(|block| block.slot_mut(index))
                else {
                    return false;
                };
                *held = Some(stack);
                self.after_commit(sunrise_gameplay_common::game::transactions::After::Changed(
                    pos,
                ));
                true
            }
            Some(
                MenuSlotBacking::Ender(_)
                | MenuSlotBacking::CraftGrid(_)
                | MenuSlotBacking::Native(_)
                | MenuSlotBacking::Owned(_),
            ) => {
                self.journal_menus(player);
                let Some(held) = self.native_menu_storage(menu, slot) else {
                    return false;
                };
                *held = Some(stack);
                true
            }
            _ => false,
        }
    }
    /// Resolve the menu's owner through the shared player boundary. The menu metadata,
    /// crafting grid and native slots stay on the guest during a turn; inventory,
    /// equipment, cursor and ender contents are swapped into Game.
    pub(in crate::game) fn native_menu_player(&self, player: PlayerRef) -> Option<&Guest> {
        match self.player_handle_read(player)? {
            sunrise_gameplay_common::game::native_server::PlayerRead::Guest(guest) => Some(guest),
            sunrise_gameplay_common::game::native_server::PlayerRead::Acting { .. } => {
                self.server.lan.as_ref()?.hub.guest(self.server.lan_acting?)
            }
        }
    }
    /// Lend the player's live slots and crafting result, expanding sparse container storage if needed.
    pub(in crate::game) fn with_live_slot_rules<R>(
        &self,
        guest: &Guest,
        backing: MenuSlotBacking,
        slot: usize,
        ask: impl FnOnce(&LiveSlotRules<'_>, &Env<'_>, Option<&ItemStack>) -> R,
    ) -> Option<R> {
        let acting = self.server.lan_acting == Some(guest.conn);
        let creative = if acting {
            self.instabuild()
        } else {
            guest.creative()
        };
        let (player_slots, player_armor, player_offhand) = if acting {
            (
                &self.common.inventory,
                &self.common.armor,
                &self.common.offhand,
            )
        } else {
            (&guest.inventory, &guest.armor, &guest.offhand)
        };
        let parts = self.menu_can_equip();
        let env = self.guest_slot_env(guest, &parts);
        let computed;
        let main_slots;
        let second_slots;
        let rules = match &guest.menu {
            None => LiveSlotRules {
                kind: Kind::Inventory,
                registry_id: Some(sunrise_sim::menu::INVENTORY_MENU),
                pos: None,
                main: &[],
                main_second: &[],
                linked: None,
                grid: &guest.craft_grid,
                result: std::slice::from_ref(&guest.craft_result),
                player_slots,
                player_armor,
                player_offhand,
                container_limit: None,
                crafter_disabled: false,
                creative,
            },
            Some(open) => {
                let world = self.native_level_world(self.level(&guest.dimension)?)?;
                let registry_id = Some(open.registry_id.as_ref());
                let entity = match open.slots {
                    Slots::Block(pos) => world.block_entities.get(&pos),
                    _ => None,
                };
                let kind = match entity.and_then(|e| machine_menu_kind(e, open.mod_type)) {
                    Some(kind) => kind,
                    None if open.slots == Slots::Native
                        && let Some(layout) = open.mod_type =>
                    {
                        Kind::Mod(layout)
                    }
                    None if registry_id == Some("minecraft:shulker_box") => Kind::Shulker,
                    None if open.size == 5 => Kind::Hopper,
                    None if open.size == 9 => Kind::Dispenser,
                    None => Kind::Chest(open.size / 9),
                };
                (main_slots, second_slots) = open.slots.shown_slices(
                    world,
                    guest,
                    acting.then_some(&self.server.ender_items[..]),
                );
                let mut main = main_slots.as_ref();
                if let Kind::Mod(layout) = kind {
                    main = &main[..main.len().min(layout.slots)];
                }
                let (grid, result) = if let Kind::Mod(layout) = kind
                    && let Some(crafting) = layout.crafting
                {
                    let grid = main.get(crafting.grid_slots()).unwrap_or_default();
                    let result = if let Some(shown) = open.shown_craft_result(grid) {
                        shown
                    } else {
                        computed = env.crafting_result(grid, crafting.size);
                        &computed
                    };
                    (grid, std::slice::from_ref(result))
                } else {
                    (&[][..], &[][..])
                };
                LiveSlotRules {
                    kind,
                    registry_id,
                    pos: open.pos.map(|(x, y, z)| [x, y, z]),
                    main,
                    main_second: second_slots.as_ref(),
                    linked: open.linked_slots(world),
                    grid,
                    result,
                    player_slots,
                    player_armor,
                    player_offhand,
                    container_limit: match backing {
                        MenuSlotBacking::BlockEntity { pos: [x, y, z], .. } => {
                            world.container_stack_limit((x, y, z))
                        }
                        _ => None,
                    },
                    crafter_disabled: matches!(entity, Some(BlockEntity::Crafter(c))
                        if c.disabled.get(slot).copied().unwrap_or(false)),
                    creative,
                }
            }
        };
        let held = match backing {
            MenuSlotBacking::Player(n @ 0..36) => (if acting {
                &self.common.inventory
            } else {
                &guest.inventory
            })
            .get(n)
            .and_then(Option::as_ref),
            MenuSlotBacking::Player(n @ 36..40) => (if acting {
                &self.common.armor
            } else {
                &guest.armor
            })[39 - n]
                .as_ref(),
            MenuSlotBacking::Player(_) => (if acting {
                &self.common.offhand
            } else {
                &guest.offhand
            })
            .as_ref(),
            _ => rules.held_container_slot(&env, slot),
        };
        Some(ask(&rules, &env, held))
    }
    /// The game's own storage menu slot `slot` shows, for a write: the ender chest's, the
    /// crafting grid's or a mod's chest menu's slot, kept on the guest. None for any other.
    pub fn native_menu_storage(
        &mut self,
        menu: MenuRef,
        slot: usize,
    ) -> Option<&mut Option<ItemStack>> {
        let backing = self.menu_slot_backing(menu, slot)?;
        if let MenuSlotBacking::Ender(n) = backing
            && self.server.lan_acting_uuid == Some(menu.fields().player.uuid())
        {
            return self.server.ender_items.get_mut(n);
        }
        let guests = &mut self.server.lan.as_mut()?.hub.guests;
        let index = guests.iter().position(|g| {
            g.placed
                && g.uuid == menu.fields().player.uuid()
                && g.entity_id as u32 == menu.fields().player.entity().id()
        })?;
        match backing {
            MenuSlotBacking::Ender(n) => guests.ender_items_mut(index)?.get_mut(n),
            MenuSlotBacking::CraftGrid(n) => guests.craft_grid_mut(index)?.get_mut(n),
            MenuSlotBacking::Native(n) | MenuSlotBacking::Owned(n) => {
                if guests[index].menu.as_ref()?.removed {
                    return None;
                }
                guests.native_menu_mut(index)?.get_mut(n)
            }
            _ => None,
        }
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
