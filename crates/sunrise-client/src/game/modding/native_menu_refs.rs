//! Handles to supported menus: shared reads, server storage and client clicks.
use super::{Game, api_values::stack_value, lan_crafting::machine_menu_kind};
use mod_api::__private::game::*;
use mod_api::{
    DragKind, EntityRef, HostTransaction, ItemResource, LevelReadError, MenuClick, MenuRef,
    MenuSlotBacking, MouseButton, PlayerRef, ResourceHandler, SwapWith,
};
use sunrise_client::{
    containers::BlockEntity,
    items::ItemStack,
    lan::{
        guest::Guest,
        menu::{SlotBacking, Slots, VEHICLE_KEY_Y},
    },
    menu::{Env, Input, Kind, LiveSlotRules},
};

impl Game<'_> {
    /// Submit clicks on the local player's live menu through normal prediction
    /// and `ContainerClick`. Slots use the same menu order as `slot`; `None`
    /// is outside (-999). All clicks are validated before any prediction or send.
    /// A drag expands to one complete quick-craft gesture.
    /// Refuses stale menus, invalid slots/hotbar indices, spectators, non-client hosts,
    /// the creative inventory screen and empty batches without changing items.
    /// True means submitted, not accepted by the server: its corrections win.
    pub(super) fn menu_click(&mut self, menu: MenuRef, clicks: &[MenuClick]) -> bool {
        if self.is_logical_server() || self.screen == super::Screen::Creative || self.spectator() {
            return false;
        }
        let Some(name) = self.native_client_menu_name(menu) else {
            return false;
        };
        let Some(ops) = client_click_ops(name, clicks, self.instabuild()) else {
            return false;
        };
        self.remote_menu_ops(&ops)
    }

    /// Close the local player's open container screen as Escape does
    /// (`AbstractContainerScreen.onClose`, `LocalPlayer.closeContainer`): the
    /// server is told (`ContainerClose`) and the screen goes. Refuses stale
    /// menus, non-client hosts and a menu whose screen is not open, such as
    /// the inventory menu while playing.
    pub(super) fn menu_close(&mut self, menu: MenuRef) -> bool {
        if self.is_logical_server()
            || self.screen != super::Screen::Inventory
            || self.native_client_menu_name(menu).is_none()
        {
            return false;
        }
        self.close_container();
        self.set_screen(super::Screen::Playing);
        self.capture(true);
        true
    }

    /// `ServerPlayer.closeContainer` of a guest's menu: queue the client's close before
    /// running normal server teardown. Refuses clients, stale menus, a guest with a pending
    /// disconnect, and the inventory menu (window 0). In the player's own turn, teardown
    /// uses their active inventory and cursor. In a transaction, true accepts a close at commit.
    pub(super) fn menu_close_container(&mut self, menu: MenuRef) -> bool {
        let window = menu.fields().window;
        if !self.is_logical_server() || window == 0 {
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

    /// `AbstractContainerMenu.setData` of a live blockless mod menu, journaled by window
    /// generation. The client reads the committed updates as live frame data, not opening bytes.
    pub(super) fn menu_set_data_slot(
        &mut self,
        menu: MenuRef,
        index: usize,
        value: i32,
    ) -> Result<bool, mod_api::MenuDataRefusal> {
        use mod_api::MenuDataRefusal::*;
        if !self.is_logical_server() {
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
            self.lan.as_mut().unwrap().hub.update_guest(conn, |guest| {
                guest.set_menu_data(index, value);
            });
        }
        Ok(true)
    }

    /// Press button `button` of the local player's open menu as a click on
    /// one of its buttons does (`MultiPlayerGameMode.handleInventoryButtonClick`,
    /// `ContainerButtonClick`): the server's menu decides what it does
    /// (`clickMenuButton`, and `menu_button` for a mod's menu). For a client
    /// mod's own input on a menu's screen other than a press, such as a key
    /// or the wheel; no sound, that is the screen's. Refuses
    /// stale menus, non-client hosts and the inventory menu, which no server
    /// opened. True means sent, not that the menu took it.
    pub(super) fn menu_click_button(&mut self, menu: MenuRef, button: i32) -> bool {
        !self.is_logical_server()
            && self.native_client_menu_name(menu).is_some()
            && self.remote_button_click(button)
    }

    pub(super) fn menu_slot_boxes(&self, menu: MenuRef) -> Option<Vec<Option<[f32; 4]>>> {
        if !matches!(
            self.screen,
            super::Screen::Inventory | super::Screen::Creative
        ) || !self.screen_hooks_open()
        {
            return None;
        }
        let name = self.native_client_menu_name(menu)?;
        let renderer = self.renderer.as_ref()?;
        let batch = super::HudBatch::new(
            [renderer.config.width, renderer.config.height],
            self.settings.gui_scale_inputs(),
        );
        self.inventory_menu_slot_boxes(menu.fields().window, name, &batch)
    }

    fn inventory_menu_slot_boxes(
        &self,
        window: i32,
        name: &str,
        batch: &super::HudBatch,
    ) -> Option<Vec<Option<[f32; 4]>>> {
        if self.book_covers_menu(batch.width) {
            return None;
        }
        let kind = self.native_screen_menu_type(window, name);
        let slot_id = |slot| Self::inventory_screen_slot_with_type(name, slot, || kind);
        match kind {
            None => {
                slot_id(0)?;
            }
            Some(kind) if kind.slot_count() == 0 => return Some(Vec::new()),
            Some(_) => {}
        }
        let visible = self.slots();
        let (ox, oy) = self.inventory_origin(batch);
        Some(
            (0..)
                .map_while(slot_id)
                .map(|slot| {
                    visible.contains(&slot).then(|| {
                        let (x, y) = self.slot_position(slot);
                        [ox + x, oy + y, 16.0, 16.0]
                    })
                })
                .collect(),
        )
    }

    pub(super) fn menu_merchant_offers(
        &self,
        menu: MenuRef,
    ) -> Result<Option<&[sunrise_client::merchant::MerchantOffer]>, LevelReadError> {
        if self.native_client_menu_name(menu) != Some("minecraft:merchant") {
            return Ok(None);
        }
        let merchant = self
            .station
            .as_ref()
            .and_then(|station| station.merchant.as_ref())
            .ok_or(LevelReadError::Unavailable)?;
        Ok(Some(&merchant.session.offers))
    }

    pub(super) fn native_client_menu_name(&self, menu: MenuRef) -> Option<&str> {
        let remote = self.remote.as_ref().filter(|remote| remote.logged_in)?;
        let epoch = self.native_lifecycle.epoch()?;
        (menu.fields().player.fields().entity.epoch() == epoch
            && menu.fields().player.fields().entity.id()
                == u32::try_from(remote.entity_id).ok()?
            && menu.fields().player.fields().uuid == remote.profile
            && menu.fields().generation == remote.menu.generation
            && menu.fields().window == remote.menu.as_ref().map_or(0, |entry| entry.0))
        .then_some(())?;
        Some(
            remote
                .menu
                .as_ref()
                .map_or("inventory", |entry| entry.1.as_str()),
        )
    }

    /// The local player in a logged-in client world.
    pub(in crate::game) fn local_player(&self) -> Option<PlayerRef> {
        let remote = self.remote.as_ref().filter(|remote| remote.logged_in)?;
        self.player(remote.profile)
    }

    /// Resolve the menu's owner through the shared player boundary. The menu metadata,
    /// crafting grid and native slots stay on the guest during a turn; inventory,
    /// equipment, cursor and ender contents are swapped into Game.
    pub(super) fn native_menu_player(&self, player: PlayerRef) -> Option<&Guest> {
        match self.player_handle_read(player)? {
            super::native_server::PlayerRead::Guest(guest) => Some(guest),
            super::native_server::PlayerRead::Acting { .. } => {
                self.lan.as_ref()?.hub.guest(self.lan_acting?)
            }
        }
    }

    pub(super) fn native_menu_guest(&self, menu: MenuRef) -> Option<&Guest> {
        let guest = self.native_menu_player(menu.fields().player)?;
        let host = self.lan.as_ref()?;
        (host.hub.holder_guest(guest.uuid, self.dim.id())?.conn == guest.conn).then_some(())?;
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

    /// A player's current supported menu. Refuses crafting tables, merchants,
    /// workstations, lecterns and mounts. Local screen hides do not change server access.
    /// Also refuses stale
    /// players and players whose state is swapped out by another active turn.
    /// The acting player's inventory, equipment, cursor and ender reads use Game's state.
    /// On a client, only the local player's supported menu is available.
    pub(super) fn player_container_menu(&self, player: PlayerRef) -> Option<MenuRef> {
        self.player_read_both(player)?;
        if !self.is_logical_server() {
            let remote = self.remote.as_ref()?;
            let (window, name) = remote
                .menu
                .as_ref()
                .map_or((0, "inventory"), |entry| (entry.0, entry.1.as_str()));
            if self.native_screen_menu_type(window, name).is_none() {
                Self::inventory_screen_slot_with_type(name, 0, || None)?;
            }
            return Some(mod_api::__private::game::menu_ref(
                player,
                window,
                remote.menu.generation,
            ));
        }
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
    pub(super) fn menu_container_id(&self, menu: MenuRef) -> Option<i32> {
        self.native_menu_guest(menu)?;
        Some(menu.fields().window)
    }

    /// Registry menu type (`AbstractContainerMenu.getType`); the inventory and
    /// mount menus have no MenuType in vanilla. The outer None means an unavailable menu.
    pub(super) fn menu_kind(&self, menu: MenuRef) -> Option<Option<String>> {
        if self.remote.is_some() {
            let name = self.native_client_menu_name(menu)?;
            return Some(
                (name != "inventory" && !name.starts_with(super::remote::MOUNT_MENU))
                    .then(|| name.to_owned()),
            );
        }
        let guest = self.native_menu_guest(menu)?;
        match &guest.menu {
            Some(m) => Some(Some(
                self.lan
                    .as_ref()?
                    .hub
                    .registry_name("minecraft:menu", m.kind)?
                    .to_owned(),
            )),
            None => Some(None),
        }
    }

    /// Number of slots in AbstractContainerMenu.slots.
    pub(super) fn menu_slot_count(&self, menu: MenuRef) -> Option<usize> {
        if self.remote.is_some() {
            let name = self.native_client_menu_name(menu)?;
            if let Some(kind) = self.native_screen_menu_type(menu.fields().window, name) {
                return Some(kind.slot_count());
            }
            let count = (0..)
                .map_while(|slot| Self::inventory_screen_slot_with_type(name, slot, || None))
                .count();
            return (count != 0).then_some(count);
        }
        let guest = self.native_menu_guest(menu)?;
        Some(
            guest
                .menu
                .as_ref()
                .map_or(46, sunrise_client::lan::menu::Menu::slot_count),
        )
    }

    /// The storage menu slot `slot` shows (`Slot.container` and `getContainerSlot`): the
    /// player's inventory by `ENTITY_ITEMS` index (`InventoryMenu`'s armour slots 5..9 go head
    /// to feet), a block entity's or an entity's slot, the ender chest's, the crafting grid's
    /// or a mod's chest menu's.
    pub(super) fn menu_slot_backing(&self, menu: MenuRef, slot: usize) -> Option<MenuSlotBacking> {
        let guest = self.native_menu_guest(menu)?;
        self.menu_slot_backing_for(guest, slot)
    }

    pub(super) fn menu_slot_backing_for(
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
        // A block menu whose block entities no longer show its slots (a chest's, over a
        // furnace set in its place) backs none of them, the player's included, until the close
        // pass ends it (`Container.stillValid`).
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
            // Only a slot a mod backed (`MenuRef::back_slots`) reads as a place.
            SlotBacking::Native(n) => match self.native_menu_place(guest.conn, open.window, n) {
                Some(place) => MenuSlotBacking::Place(place),
                None if open.owned_slots().is_some() => MenuSlotBacking::Owned(n),
                None => MenuSlotBacking::Native(n),
            },
        })
    }

    /// Full stack at a menu index. Outer None means unavailable or out of
    /// range; Some(None) means an empty slot.
    pub(super) fn menu_slot(&self, menu: MenuRef, slot: usize) -> Option<Option<ItemStack>> {
        if self.remote.is_some() {
            let name = self.native_client_menu_name(menu)?;
            let index = Self::inventory_screen_slot(name, i32::try_from(slot).ok()?)?;
            return Some(self.slot_view(index));
        }
        let guest = self.native_menu_guest(menu)?;
        let backing = self.menu_slot_backing_for(guest, slot)?;
        let acting = self.lan_acting == Some(guest.conn);
        Some(match backing {
            MenuSlotBacking::BlockEntity { .. } | MenuSlotBacking::Entity { .. } => {
                let world = self.native_level_world(self.level(&guest.dimension)?)?;
                guest.menu.as_ref()?.slots.get(world, guest, slot)
            }
            MenuSlotBacking::Player(n @ 0..36) => {
                if acting {
                    self.inventory.get(n)?.clone()
                } else {
                    guest.inventory.get(n)?.clone()
                }
            }
            MenuSlotBacking::Player(n @ 36..40) => {
                if acting {
                    self.armor[39 - n].clone()
                } else {
                    guest.armor[39 - n].clone()
                }
            }
            MenuSlotBacking::Player(_) => {
                if acting {
                    self.offhand.clone()
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
                    self.ender_items.get(n)?.clone()
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

    /// How many of `stack` menu slot `slot` holds (`Slot.mayPlace`, then `Slot.getMaxStackSize`):
    /// a furnace's fuel slot one bucket, an armour slot one piece that fits it, a crafting result
    /// none. Returns `None` for enabled handler-backed slots (`MenuSlotBacking::Place`), which
    /// need [`Self::menu_probe_slot_holds`].
    pub(super) fn menu_slot_holds(
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

    /// Whether menu slot `slot` gives up what it holds (`Slot.mayPickup`): bound armour stays
    /// on outside creative. Returns `None` for enabled handler-backed slots
    /// (`MenuSlotBacking::Place`), which need [`Self::menu_probe_slot_gives`].
    pub(super) fn menu_slot_gives(&self, menu: MenuRef, slot: usize) -> Option<bool> {
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
    pub(super) fn menu_probe_slot_holds(
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
    pub(super) fn menu_probe_slot_gives(
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
        let size = self.lan.as_ref()?.hub.guest(conn)?.menu.as_ref()?.size;
        Some(self.place_menu_rules(conn, size, locked, slot, resource, amount))
    }

    pub(super) fn menu_slot_gives_with_stack(
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

    pub(super) fn menu_slot_holds_with_stack(
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

    pub(super) fn menu_owned_slot_limit(&self, menu: MenuRef, slot: usize) -> Option<u32> {
        let open = self.native_menu_guest(menu)?.menu.as_ref()?;
        let limit = *open.owned_slots()?.get(slot)?;
        Some(u32::from(limit))
    }

    /// Lend the admitted player's live slots and crafting result to a rule ask, without
    /// constructing a scratch menu or cloning the container, inventory, or cursor.
    pub(super) fn with_live_slot_rules<R>(
        &self,
        guest: &Guest,
        backing: MenuSlotBacking,
        slot: usize,
        ask: impl FnOnce(&LiveSlotRules<'_>, &Env<'_>, Option<&ItemStack>) -> R,
    ) -> Option<R> {
        let acting = self.lan_acting == Some(guest.conn);
        let creative = if acting {
            self.instabuild()
        } else {
            guest.creative()
        };
        let (player_slots, player_armor, player_offhand) = if acting {
            (&self.inventory, &self.armor, &self.offhand)
        } else {
            (&guest.inventory, &guest.armor, &guest.offhand)
        };
        let parts = self.menu_can_equip();
        let env = self.guest_slot_env(guest, &parts);
        let computed;
        let rules = match &guest.menu {
            None => LiveSlotRules {
                kind: Kind::Inventory,
                registry_id: None,
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
                let (mut main, main_second) =
                    open.slots
                        .shown_slices(world, guest, acting.then_some(&self.ender_items[..]));
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
                    main_second,
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
                &self.inventory
            } else {
                &guest.inventory
            })
            .get(n)
            .and_then(Option::as_ref),
            MenuSlotBacking::Player(n @ 36..40) => {
                (if acting { &self.armor } else { &guest.armor })[39 - n].as_ref()
            }
            MenuSlotBacking::Player(_) => (if acting {
                &self.offhand
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
    pub(super) fn native_menu_storage(
        &mut self,
        menu: MenuRef,
        slot: usize,
    ) -> Option<&mut Option<ItemStack>> {
        let backing = self.menu_slot_backing(menu, slot)?;
        if let MenuSlotBacking::Ender(n) = backing
            && self.lan_acting_uuid == Some(menu.fields().player.uuid())
        {
            return self.ender_items.get_mut(n);
        }
        let guests = &mut self.lan.as_mut()?.hub.guests;
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

    /// Write back an already-held engine stack after in-place item processing, without
    /// routing it through insertion rules or changing its count. Refuses virtual previews
    /// and handler-owned slots: their display mirrors do not own the item.
    pub(super) fn replace_crafted_menu_slot(
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
                        .held_block_slot(&self.world, guest, slot)
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
                    .world
                    .block_entities
                    .get_mut(&pos)
                    .and_then(|block| block.slot_mut(index))
                else {
                    return false;
                };
                *held = Some(stack);
                self.after_commit(super::transactions::After::Changed(pos));
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

    /// `AbstractContainerMenu.getStateId`: on a client the value the server last sent for the
    /// open menu, on a server the open menu's counter or the inventory menu's.
    pub(super) fn menu_state_id(&self, menu: MenuRef) -> Option<i32> {
        if let Some(remote) = self.remote.as_ref() {
            self.native_client_menu_name(menu)?;
            return Some(remote.state_id);
        }
        let guest = self.native_menu_guest(menu)?;
        Some(
            guest
                .menu
                .as_ref()
                .map_or(guest.menu_state, |open| open.state),
        )
    }

    /// Carried (cursor) stack; outer None means unavailable.
    pub(super) fn menu_carried(&self, menu: MenuRef) -> Option<Option<ItemStack>> {
        if self.remote.is_some() {
            self.native_client_menu_name(menu)?;
            return Some(self.carried.clone());
        }
        let guest = self.native_menu_guest(menu)?;
        if Self::menu_cursor_retired(guest) {
            return Some(None);
        }
        Some(if self.lan_acting == Some(guest.conn) {
            self.carried.clone()
        } else {
            guest.carried.clone()
        })
    }

    /// Whether the guest's open menu has been removed: its cursor went back to the inventory's
    /// own, which the closing handle must not read or edit (`AbstractContainerMenu.removed`
    /// leaves the closing menu with no carried stack).
    pub(super) fn menu_cursor_retired(guest: &Guest) -> bool {
        guest.menu.as_ref().is_some_and(|open| open.removed)
    }
}
fn client_click_ops(
    menu: &str,
    clicks: &[MenuClick],
    creative: bool,
) -> Option<Vec<(Option<usize>, i32, Input)>> {
    if clicks.is_empty() {
        return None;
    }
    let mapped_slot =
        |slot| Game::inventory_screen_slot(menu, i32::from(i16::try_from(slot).ok()?));
    let mut ops = Vec::new();
    for click in clicks {
        let op = match click {
            MenuClick::Pickup { slot, button } => (
                match slot {
                    Some(slot) => Some(mapped_slot(*slot)?),
                    None => None,
                },
                match button {
                    MouseButton::Left => 0,
                    MouseButton::Right => 1,
                },
                Input::Pickup,
            ),
            MenuClick::QuickMove { slot } => (Some(mapped_slot(*slot)?), 0, Input::QuickMove),
            MenuClick::Swap { slot, with } => (
                Some(mapped_slot(*slot)?),
                match with {
                    SwapWith::Hotbar(index) if *index <= 8 => i32::from(*index),
                    SwapWith::Offhand => 40,
                    _ => return None,
                },
                Input::Swap,
            ),
            MenuClick::Clone { slot } if creative => (Some(mapped_slot(*slot)?), 2, Input::Clone),
            MenuClick::Throw { slot, whole_stack } => (
                Some(mapped_slot(*slot)?),
                i32::from(*whole_stack),
                Input::Throw,
            ),
            MenuClick::Drag { kind, slots } => {
                let mode = match kind {
                    DragKind::Split => 0,
                    DragKind::One => 4,
                    DragKind::Clone if creative => 8,
                    _ => return None,
                };
                ops.push((None, mode, Input::QuickCraft));
                for slot in slots {
                    ops.push((Some(mapped_slot(*slot)?), mode + 1, Input::QuickCraft));
                }
                ops.push((None, mode + 2, Input::QuickCraft));
                continue;
            }
            MenuClick::PickupAll { slot } => (Some(mapped_slot(*slot)?), 0, Input::PickupAll),
            _ => return None,
        };
        ops.push(op);
    }
    Some(ops)
}

#[cfg(test)]
mod tests {
    use super::super::api_values::TryIntoApi;
    use super::super::{lan::LanSettings, native_entity_refs::tests::with_game};
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicI32, AtomicUsize},
    };
    use sunrise_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
    use sunrise_protocol::packets::configuration::ClientInformation;

    const PANEL: &str = "test:panel";

    struct Panel {
        opening: Arc<Mutex<Option<(MenuRef, usize)>>>,
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Panel {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.register_menu_screen(
                PANEL,
                Arc::new(
                    |_: &dyn mod_api::ClientHost, open: mod_api::MenuScreenOpen<'_>| {
                        Some(mod_api::MenuScreen::new(mod_api::MenuScreenLayout {
                            width: 176.0,
                            height: 166.0,
                            slots: (0..open.backing_slots)
                                .map(|index| {
                                    Some(mod_api::ScreenSlot {
                                        position: [
                                            8.0 + (index % 9) as f32 * 18.0,
                                            18.0 + (index / 9) as f32 * 18.0,
                                        ],
                                        no_item_icon: None,
                                    })
                                })
                                .collect(),
                            ..Default::default()
                        }))
                    },
                ),
            )
            .unwrap();
        }

        fn screen_opening(
            &self,
            host: &dyn mod_api::ClientHost,
            event: &mut mod_api::ScreenOpeningEvent<'_>,
        ) -> Option<mod_api::ScreenOpening> {
            if event.current.id == "playing" && event.new.menu == Some(PANEL) {
                let menu = host.local_player()?.container_menu(host)?;
                let count = menu.slot_count(host)?;
                *self.opening.lock().unwrap() = Some((menu, count));
            }
            None
        }
    }

    fn with_panel(
        slots: usize,
        test: impl FnOnce(&mut Game<'_>, &Mutex<Option<(MenuRef, usize)>>),
    ) {
        let opening = Arc::new(Mutex::new(None));
        let panel = Panel {
            opening: opening.clone(),
        };
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("test", Box::new(panel))].into();
        mod_platform::testing::with_mods(mods.clone(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
            sunrise_client::containers::with_mod_menu_types(
                [(
                    PANEL.into(),
                    "test".into(),
                    slots,
                    0,
                    vec![],
                    vec![],
                    None,
                    None,
                    false,
                    None,
                )],
                || {
                    super::super::native_entity_refs::tests::with_client_game(|game| {
                        let _server =
                            super::super::native_client_session::tests::logged_client(game);
                        Arc::make_mut(&mut game.remote.as_mut().unwrap().builtin)
                            .registries
                            .insert(
                                "minecraft:menu".into(),
                                sunrise_protocol::registry::Ids::from_names(vec![PANEL.into()]),
                            );
                        test(game, &opening);
                    })
                },
            );
        });
    }

    fn open_panel(game: &mut Game<'_>) {
        use sunrise_protocol::packets::play::{Clientbound, OpenScreen};
        game.handle_packet(Clientbound::OpenScreen(OpenScreen {
            container: 1,
            menu: 0,
            title: sunrise_core::nbt::Tag::String("Panel".into()),
        }))
        .unwrap();
        assert!(game.native_screen_shown());
    }

    #[test]
    #[ignore = "daily"]
    fn inventory_boxes_follow_menu_indices_and_the_input_layout() {
        with_panel(0, |game, opening| {
            game.screen = super::super::Screen::Inventory;
            let batch = super::super::HudBatch::new([800, 600], game.settings.gui_scale_inputs());
            let boxes = game
                .inventory_menu_slot_boxes(0, "inventory", &batch)
                .unwrap();
            assert_eq!(boxes.len(), 46);
            for (index, expected_slot) in [(0, 200), (9, 9), (36, 0), (45, 504)] {
                let [x, y, w, h] = boxes[index].unwrap();
                assert_eq!(
                    game.slot_at(&batch, x + w / 2.0, y + h / 2.0),
                    Some(expected_slot)
                );
            }
            assert!(game.inventory_menu_slot_boxes(1, PANEL, &batch).is_none());
            game.screen = super::super::Screen::Playing;
            open_panel(game);
            let menu = game
                .player_container_menu(game.local_player().unwrap())
                .unwrap();
            assert_eq!(
                *opening.lock().unwrap(),
                Some((menu, 0)),
                "the same live handle and count are available during screen_opening"
            );
            assert!(
                game.inventory_menu_slot_boxes(1, "unsupported", &batch)
                    .is_none()
            );
            assert_eq!(
                game.inventory_menu_slot_boxes(1, PANEL, &batch),
                Some(vec![])
            );
            assert_eq!(
                game.menu_set_data_slot(menu, 0, 1),
                Err(mod_api::MenuDataRefusal::Unsupported)
            );
        });
    }

    fn creative_tab(game: &mut Game<'_>, pick: impl Fn(&sunrise_client::creative::Tab) -> bool) {
        let tab = game.creative_screen.tabs.iter().position(pick).unwrap();
        game.creative_screen.select(tab);
    }

    /// A hidden window's offscreen renderer over a joined level with the player's body in it,
    /// so the public screen reads have a window to lay the screen out in. The event loop
    /// returned holds the window.
    fn offscreen_window(game: &mut Game<'_>) -> winit::event_loop::EventLoop<()> {
        let (event_loop, window) = crate::game::native_entity_refs::tests::test_window([320, 240]);
        game.options.offscreen = true;
        {
            let renderer = game.new_renderer(window).unwrap();
            game.set_renderer(renderer);
        }
        let remote = game.remote.as_mut().unwrap();
        remote.level_load.finish();
        let mut body = sunrise_client::motion::PlayerMotion::new(
            game.motion,
            sunrise_core::geometry::Vec3::ZERO,
        )
        .unwrap();
        body.entity_id = remote.entity_id;
        game.player = Some(body);
        event_loop
    }

    fn window_batch(game: &Game<'_>) -> super::super::HudBatch {
        let config = &game.renderer.as_ref().unwrap().config;
        super::super::HudBatch::new(
            [config.width, config.height],
            game.settings.gui_scale_inputs(),
        )
    }

    /// The creative screen's boxes, read through `MenuRef::slot_boxes`, are `InventoryMenu`'s
    /// 46 slots where `CreativeModeInventoryScreen.selectTab` puts them, and each box's centre
    /// hovers its slot.
    #[test]
    #[ignore = "daily"]
    fn creative_boxes_follow_the_selected_tab() {
        crate::game::in_own_process(
            "game::native_menu_refs::tests::creative_boxes_follow_the_selected_tab",
            || {
                with_panel(0, |game, _| {
                    let _window = offscreen_window(game);
                    game.creative = true;
                    game.screen = super::super::Screen::Creative;
                    game.refresh_creative_tabs();
                    let batch = window_batch(game);
                    let (ox, oy) = Game::creative_origin(&batch);
                    // Menu index, game slot, position on the inventory tab, shown on item tabs.
                    let expected = |index: usize| -> Option<(usize, (f32, f32), bool)> {
                        let row = |i: usize| (9.0 + (i % 9) as f32 * 18.0, (i / 9) as f32 * 18.0);
                        match index {
                            5..=8 => {
                                let i = index - 5;
                                let at =
                                    (54.0 + (i / 2) as f32 * 54.0, 6.0 + (i % 2) as f32 * 27.0);
                                Some((500 + i, at, false))
                            }
                            9..=35 => {
                                let (x, y) = row(index - 9);
                                Some((index, (x, 54.0 + y), false))
                            }
                            36..=44 => Some((index - 36, (row(index - 36).0, 112.0), true)),
                            45 => Some((504, (35.0, 20.0), false)),
                            _ => None,
                        }
                    };
                    for tab in ["inventory", "items", "search"] {
                        creative_tab(game, |t| match tab {
                            "inventory" => t.inventory,
                            "items" => !t.inventory && !t.search && !t.items.is_empty(),
                            _ => t.search,
                        });
                        let host: &dyn mod_api::ClientHost = &*game;
                        let menu = host.local_player().unwrap().container_menu(host).unwrap();
                        let boxes = menu.slot_boxes(host).unwrap();
                        assert_eq!(boxes.len(), 46, "{tab}");
                        for (index, found) in boxes.into_iter().enumerate() {
                            let want = expected(index)
                                .filter(|&(_, _, item_tab)| tab == "inventory" || item_tab);
                            assert_eq!(
                                found,
                                want.map(|(_, (x, y), _)| [ox + x, oy + y, 16.0, 16.0]),
                                "{tab} slot {index}"
                            );
                            if let Some((slot, (x, y), _)) = want {
                                assert_eq!(
                                    game.slot_at(&batch, ox + x + 8.0, oy + y + 8.0),
                                    Some(slot),
                                    "{tab} slot {index}"
                                );
                            }
                        }
                    }
                })
            },
        );
    }

    /// `screen_hovered_stack` gives a mod the hovered slot's whole stack, components included,
    /// on the inventory screen and on both kinds of creative tab, where a catalog slot shows
    /// the scrolled list; nothing over an empty slot, the trash slot or the world.
    #[test]
    #[ignore = "daily"]
    fn the_hovered_stack_keeps_its_components() {
        crate::game::in_own_process(
            "game::native_menu_refs::tests::the_hovered_stack_keeps_its_components",
            || {
                use mod_api::{Tag, prelude::ScreensClientHost};
                use sunrise_client::creative::Creative;
                use sunrise_core::nbt::JavaString;

                fn component<'s>(stack: &'s mod_api::ItemStack, id: &str) -> Option<&'s Tag> {
                    stack.components()?.as_nbt().get(&JavaString::from(id))
                }
                let key = |id| mod_api::__private::item_key(id).unwrap();
                let is_sword = |stack: Option<mod_api::ItemStack>| {
                    let stack = stack.expect("the sword");
                    assert_eq!(stack.item(), key("minecraft:diamond_sword"));
                    assert_eq!(stack.count(), 1);
                    assert_eq!(stack.damage_component(), Some(5));
                    assert_eq!(
                        component(&stack, "minecraft:enchantments"),
                        Some(&Tag::compound([("minecraft:sharpness", Tag::Int(3))]))
                    );
                    assert_eq!(
                        component(&stack, "minecraft:custom_name"),
                        Some(&Tag::String("Blade".into()))
                    );
                };
                let is_potion = |stack: Option<mod_api::ItemStack>| {
                    let stack = stack.expect("the potion");
                    assert_eq!(stack.item(), key("minecraft:potion"));
                    assert_eq!(stack.count(), 1);
                    assert_eq!(
                        component(&stack, "minecraft:potion_contents"),
                        Some(&Tag::compound([(
                            "potion",
                            Tag::String("minecraft:strong_healing".into())
                        )]))
                    );
                };
                let is_stone = |stack: Option<mod_api::ItemStack>| {
                    let stack = stack.expect("the stone");
                    assert_eq!(stack.item(), key("minecraft:stone"));
                    assert_eq!(stack.count(), 17);
                    assert!(stack.components().is_none());
                };
                with_panel(0, |game, _| {
                    let _window = offscreen_window(game);
                    let batch = window_batch(game);
                    let hover = |game: &mut Game<'_>, slot: usize| {
                        let (ox, oy) = game.inventory_origin(&batch);
                        let (x, y) = game.slot_position(slot);
                        game.cursor = (ox + x + 8.0, oy + y + 8.0);
                        ScreensClientHost::screen_hovered_stack(&*game)
                    };
                    use crate::game::api_values::TryIntoEngine;
                    let sword = game
                        .item_from_snbt(r#"{id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":5,"minecraft:enchantments":{"minecraft:sharpness":3},"minecraft:custom_name":"Blade"}}"#)
                        .unwrap()
                        .try_into_engine(game.items())
                        .unwrap();
                    let potion = game
                        .item_from_snbt(r#"{id:"minecraft:potion",count:1,components:{"minecraft:potion_contents":{potion:"minecraft:strong_healing"}}}"#)
                        .unwrap()
                        .try_into_engine(game.items())
                        .unwrap();
                    game.inventory[0] = Some(sword.clone());
                    game.inventory[9] = Some(potion.clone());
                    game.inventory[10] = Some(ItemStack::new("minecraft:stone", 17));
                    game.inventory[11] = None;
                    game.armor[0] = Some(sword.clone());

                    game.screen = super::super::Screen::Inventory;
                    is_potion(hover(game, 9));
                    is_stone(hover(game, 10));
                    assert_eq!(hover(game, 11), None);
                    game.screen = super::super::Screen::Playing;
                    assert_eq!(hover(game, 9), None);

                    game.creative = true;
                    game.screen = super::super::Screen::Creative;
                    game.refresh_creative_tabs();
                    creative_tab(game, |t| t.inventory);
                    is_potion(hover(game, 9));
                    is_sword(hover(game, 500));
                    assert_eq!(hover(game, 11), None);
                    let (ox, oy) = game.inventory_origin(&batch);
                    let (tx, ty) = Creative::TRASH_POSITION;
                    game.cursor = (ox + tx + 8.0, oy + ty + 8.0);
                    assert_eq!(ScreensClientHost::screen_hovered_stack(&*game), None);

                    // Seven rows: scrolled to the bottom, the first shown row is the third.
                    creative_tab(game, |t| !t.inventory && !t.search && !t.items.is_empty());
                    let tab = game.creative_screen.selected_index();
                    let mut items = vec![ItemStack::new("minecraft:stone", 17); 63];
                    items[0] = sword.clone();
                    items[1].count = 0;
                    items[18] = potion;
                    game.creative_screen.tabs[tab].items = items;
                    game.creative_screen.forget_items();
                    is_sword(hover(game, Creative::DISPLAY));
                    assert_eq!(hover(game, Creative::DISPLAY + 1), None);
                    is_stone(hover(game, Creative::DISPLAY + 2));
                    is_sword(hover(game, 0));
                    game.creative_screen.scroll = 1.0;
                    is_potion(hover(game, Creative::DISPLAY));

                    game.creative_screen = Creative::from_json(&serde_json::json!({
                        "schema": 4, "item_features": {}, "potion_features": {}, "tabs": [{
                            "id": "minecraft:search", "type": "SEARCH", "title": "Search Items",
                            "build": [], "search_metadata": [{
                                "stack": r#"{id:"minecraft:stone",count:17}"#,
                                "ungrouped": true
                            }]
                        }]
                    }))
                    .unwrap();
                    is_stone(hover(game, Creative::DISPLAY));
                    is_sword(hover(game, 0));
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn number_key_swaps_predict_with_the_undisplayed_hotbar() {
        use crate::game::inventory_slots::MOD_MENU_SLOT_BASE;

        with_panel(1, |game, _| {
            open_panel(game);
            let slot = MOD_MENU_SLOT_BASE;
            assert_eq!(game.slots(), [slot]);
            let held = ItemStack::new("minecraft:stone", 5);
            let stored = ItemStack::new("minecraft:dirt", 2);
            game.inventory[2] = Some(held.clone());
            *game.slot_mut(slot) = Some(stored.clone());
            assert!(game.menu_ops(&[(Some(slot), 2, Input::Swap)]));
            assert_eq!(game.inventory[2], Some(stored));
            assert_eq!(*game.slot_mut(slot), Some(held));
            assert_eq!(game.slots(), [slot]);
        });
    }

    #[test]
    fn client_click_actions_encode_vanilla_ops() {
        let actions = [
            MenuClick::Pickup {
                slot: Some(36),
                button: MouseButton::Left,
            },
            MenuClick::Pickup {
                slot: None,
                button: MouseButton::Right,
            },
            MenuClick::QuickMove { slot: 9 },
            MenuClick::Swap {
                slot: 9,
                with: SwapWith::Hotbar(8),
            },
            MenuClick::Swap {
                slot: 9,
                with: SwapWith::Offhand,
            },
            MenuClick::Clone { slot: 9 },
            MenuClick::Throw {
                slot: 9,
                whole_stack: false,
            },
            MenuClick::Throw {
                slot: 9,
                whole_stack: true,
            },
            MenuClick::PickupAll { slot: 9 },
        ];
        assert_eq!(
            client_click_ops("inventory", &actions, true),
            Some(vec![
                (Some(0), 0, Input::Pickup),
                (None, 1, Input::Pickup),
                (Some(9), 0, Input::QuickMove),
                (Some(9), 8, Input::Swap),
                (Some(9), 40, Input::Swap),
                (Some(9), 2, Input::Clone),
                (Some(9), 0, Input::Throw),
                (Some(9), 1, Input::Throw),
                (Some(9), 0, Input::PickupAll),
            ])
        );
        for (kind, mode, creative) in [
            (DragKind::Split, 0, false),
            (DragKind::One, 4, false),
            (DragKind::Clone, 8, true),
        ] {
            assert_eq!(
                client_click_ops(
                    "inventory",
                    &[
                        MenuClick::Drag {
                            kind,
                            slots: vec![36, 9]
                        },
                        MenuClick::QuickMove { slot: 9 },
                    ],
                    creative
                ),
                Some(vec![
                    (None, mode, Input::QuickCraft),
                    (Some(0), mode + 1, Input::QuickCraft),
                    (Some(9), mode + 1, Input::QuickCraft),
                    (None, mode + 2, Input::QuickCraft),
                    (Some(9), 0, Input::QuickMove),
                ])
            );
        }
    }

    #[test]
    fn client_click_actions_refuse_invalid_batches_before_prediction() {
        assert!(client_click_ops("inventory", &[], false).is_none());
        for invalid in [
            MenuClick::QuickMove { slot: 46 },
            MenuClick::QuickMove { slot: usize::MAX },
            MenuClick::Swap {
                slot: 9,
                with: SwapWith::Hotbar(9),
            },
            MenuClick::Swap {
                slot: 9,
                with: SwapWith::Hotbar(u8::MAX),
            },
            MenuClick::Clone { slot: 9 },
            MenuClick::Drag {
                kind: DragKind::Clone,
                slots: vec![9],
            },
            MenuClick::Drag {
                kind: DragKind::Split,
                slots: vec![9, usize::MAX],
            },
        ] {
            assert!(
                client_click_ops(
                    "inventory",
                    &[MenuClick::QuickMove { slot: 9 }, invalid,],
                    false
                )
                .is_none()
            );
        }
        assert!(
            client_click_ops("unsupported", &[MenuClick::QuickMove { slot: 0 }], true).is_none()
        );
    }

    fn guest() -> Guest {
        let (tx, _) = std::sync::mpsc::channel();
        let mut g = Guest::new(
            Joined {
                conn: 1,
                uuid: 123,
                name: "guest".into(),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            1,
            [0.0; 3],
            10,
        );
        g.placed = true;
        g
    }

    #[test]
    #[ignore = "daily"]
    fn a_server_closes_a_guests_menu_and_sets_their_selected_slot() {
        with_game(|game| {
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 4,
                simulation_distance: 4,
                dimension: "minecraft:overworld".into(),
                allow_flight: true,
                seed: 0,
                max_players: 10,
                integrated: true,
                levels: game.level_shapes.clone(),
                level_seeds: game.levels.runtime.seeds(),
            };
            let mut net = NetConfig::lan(0, "test".into(), 10);
            net.listen = false;
            game.lan_host(
                LanSettings {
                    port: 0,
                    guest_commands: false,
                    force_game_mode: false,
                },
                net,
                config,
            )
            .unwrap();
            game.lan.as_mut().unwrap().hub.guests.push(guest());
            let player = game.player(123).unwrap();

            // The selected slot: a slot past the hotbar is refused, a real one is held.
            assert_eq!(player.selected_slot(game), Some(0));
            assert!(!player.set_selected_slot(game, 9));
            assert!(player.set_selected_slot(game, 4));
            assert!(
                player.set_selected_slot(game, 4),
                "the same slot is no error"
            );
            assert_eq!(player.selected_slot(game), Some(4));
            assert_eq!(game.lan.as_ref().unwrap().hub.guests[0].selected, 4);

            // A menu: closing it returns the carried stack and leaves no menu behind.
            let stone = ItemStack::new("minecraft:stone", 4);
            let host = game.lan.as_mut().unwrap();
            host.hub
                .open_native_menu(
                    1,
                    1,
                    sunrise_core::nbt::Tag::String("test".into()),
                    vec![Some(stone.clone())],
                )
                .unwrap();
            host.hub
                .guests
                .update(0, |guest| guest.carried = Some(stone.clone()));
            let menu = game.player_container_menu(player).unwrap();
            assert_ne!(menu.container_id(game), Some(0));
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| guest.disconnect_pending = true);
            assert!(!menu.close_container(game), "disconnect owns the teardown");
            assert_eq!(game.player_container_menu(player), Some(menu));
            let hub = &mut game.lan.as_mut().unwrap().hub;
            assert_eq!(hub.guests[0].carried.as_ref(), Some(&stone));
            hub.guests
                .update(0, |guest| guest.disconnect_pending = false);
            game.as_guest(1, |game| {
                assert_eq!(player.container_menu(game), Some(menu));
                assert!(menu.close_container(game));
                assert!(
                    !menu.close_container(game),
                    "the generation retired in the turn"
                );
                assert!(game.carried.is_none(), "close returned the live cursor");
                assert_eq!(
                    game.inventory
                        .iter()
                        .flatten()
                        .map(|s| s.count)
                        .sum::<u32>(),
                    4
                );
            })
            .unwrap();
            let guest = &game.lan.as_ref().unwrap().hub.guests[0];
            assert!(guest.menu.is_none(), "the menu is gone");
            assert!(guest.carried.is_none(), "the cursor stack went back");
            assert_eq!(
                guest
                    .inventory
                    .iter()
                    .flatten()
                    .map(|s| s.count)
                    .sum::<u32>(),
                4,
                "to the inventory, none lost or doubled"
            );
            assert!(!menu.close_container(game), "a stale menu is refused");
            // The player's own inventory menu is never closed.
            let inventory = game.player_container_menu(player).unwrap();
            assert_eq!(inventory.container_id(game), Some(0));
            assert!(!inventory.close_container(game));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_stack_is_thrown_from_the_eyes_as_player_drop_does() {
        use sunrise_client::motion::PlayerMotion;
        use sunrise_core::geometry::Vec3;

        with_game(|game| {
            // A body to throw from: `with_game` makes none.
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
            let stone = ItemStack::new("minecraft:stone", 3);
            let before = game.dropped.len();
            let id = game.drop_stack_from_eyes(stone.clone(), true).unwrap();
            assert_eq!(game.dropped.len(), before + 1);
            let thrown = game.dropped.last().unwrap();
            assert_eq!((thrown.id, &thrown.stack), (id, &stone));
            assert!(thrown.thrower.is_some(), "the drop key names the thrower");
            game.drop_stack_from_eyes(stone.clone(), false).unwrap();
            assert!(game.dropped.last().unwrap().thrower.is_none());
            assert_eq!(
                game.drop_stack_from_eyes(ItemStack::new("minecraft:stone", 0), true),
                None,
                "an empty stack is no drop"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn menus_read_storage_and_invalidate_on_replacement_wrap_and_reconnect() {
        with_game(|game| {
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 4,
                simulation_distance: 4,
                dimension: "minecraft:overworld".into(),
                allow_flight: true,
                seed: 0,
                max_players: 10,
                integrated: true,
                levels: game.level_shapes.clone(),
                level_seeds: game.levels.runtime.seeds(),
            };
            let mut net = NetConfig::lan(0, "test".into(), 10);
            net.listen = false;
            game.lan_host(
                LanSettings {
                    port: 0,
                    guest_commands: false,
                    force_game_mode: false,
                },
                net,
                config,
            )
            .unwrap();
            game.lan.as_mut().unwrap().hub.guests.push(guest());
            let player = game.player(123).unwrap();
            let inventory = game.player_container_menu(player).unwrap();
            assert_eq!(inventory.container_id(game), Some(0));
            assert_eq!(inventory.kind(game), Some(None));
            assert_eq!(inventory.slot_count(game), Some(46));
            assert_eq!(inventory.slot(game, 0), Some(None));
            assert_eq!(
                game.menu_slot_backing(inventory, 36),
                Some(MenuSlotBacking::Player(0))
            );
            assert_eq!(
                game.menu_slot_backing(inventory, 45),
                Some(MenuSlotBacking::Player(40))
            );
            assert_eq!(inventory.slot(game, 46), None);
            let stone = ItemStack::new("minecraft:stone", 4);
            let host = game.lan.as_mut().unwrap();
            {
                let value = Some(stone.clone());
                host.hub.guests.update(0, |g| g.inventory[9] = value);
            }
            {
                let value = Some(stone.clone());
                host.hub.guests.update(0, |guest| guest.carried = value);
            }
            let title = sunrise_core::nbt::Tag::String("test".into());
            let window = host
                .hub
                .open_native_menu(1, 1, title.clone(), vec![Some(stone.clone())])
                .unwrap();
            assert_eq!(inventory.slot_count(game), None);
            let menu = game.player_container_menu(player).unwrap();
            assert_eq!(menu.container_id(game), Some(window));
            assert_eq!(menu.kind(game), Some(Some("minecraft:generic_9x1".into())));
            assert_eq!(menu.slot_count(game), Some(45));
            assert_eq!(
                menu.slot(game, 0),
                Some(Some(stone.try_into_api().unwrap()))
            );
            assert_eq!(menu.slot(game, 1), Some(None));
            assert_eq!(
                menu.slot(game, 9),
                Some(Some(stone.try_into_api().unwrap()))
            );
            // Like a new AbstractContainerMenu, this window starts with an empty cursor,
            // independent of the inventory menu's carried stack.
            assert_eq!(menu.carried(game), Some(None));
            let stick = ItemStack::new("minecraft:stick", 2);
            game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                guest.carried = Some(stick.clone());
            });
            assert_eq!(
                menu.carried(game),
                Some(Some(stick.try_into_api().unwrap()))
            );
            assert_eq!(
                game.menu_slot_backing(menu, 0),
                Some(MenuSlotBacking::Native(0))
            );
            assert_eq!(
                game.menu_slot_backing(menu, 9),
                Some(MenuSlotBacking::Player(9))
            );
            assert_eq!(
                game.menu_slot_backing(menu, 36),
                Some(MenuSlotBacking::Player(0))
            );
            assert_eq!(game.menu_slot_backing(menu, 45), None);
            let host = game.lan.as_mut().unwrap();
            for _ in 0..100 {
                host.hub
                    .open_native_menu(1, 1, title.clone(), vec![])
                    .unwrap();
            }
            assert_eq!(host.hub.guests[0].menu.as_ref().unwrap().window, window);
            assert_eq!(menu.slot_count(game), None);
            let newer = game.player_container_menu(player).unwrap();
            let host = game.lan.as_mut().unwrap();
            host.hub.close_container(1);
            assert_eq!(newer.slot_count(game), None);
            let inventory = game.player_container_menu(player).unwrap();
            assert_eq!(
                inventory.carried(game),
                Some(Some(stone.try_into_api().unwrap()))
            );
            let host = game.lan.as_mut().unwrap();
            *host.menus.entry(1).or_default().crafting.table = Some((
                7,
                0,
                Box::new(sunrise_client::menu::Menu::new(
                    sunrise_client::menu::Kind::Crafting,
                )),
                [0; 3],
            ));
            assert_eq!(game.player_container_menu(player), None);
            assert_eq!(inventory.slot_count(game), None);
            let host = game.lan.as_mut().unwrap();
            host.play.clear();
            host.menus.clear();
            host.hub.guests.replace(0, guest());
            assert_eq!(inventory.slot_count(game), None);

            use sunrise_client::{containers::BlockEntity, lan::menu::Slots};
            let a = (1, 2, 3);
            let b = (2, 2, 3);
            let stone = ItemStack::new("minecraft:stone", 3);
            game.world
                .block_entities
                .insert(a, BlockEntity::Chest(vec![None; 2]));
            game.world
                .block_entities
                .insert(b, BlockEntity::Chest(vec![Some(stone.clone()); 3]));
            game.lan.as_mut().unwrap().hub.open_container(
                1,
                "minecraft:generic_9x1",
                title.clone(),
                a,
                Slots::Double(a, b),
                vec![None; 5],
                None,
            );
            let menu = game.player_container_menu(player).unwrap();
            assert_eq!(
                game.menu_slot_backing(menu, 2),
                Some(MenuSlotBacking::BlockEntity {
                    level: game.level("minecraft:overworld").unwrap(),
                    pos: [b.0, b.1, b.2],
                    slot: 0
                })
            );
            assert_eq!(
                menu.slot(game, 2),
                Some(Some(stone.try_into_api().unwrap()))
            );
            let vehicle = (i32::MAX - 15, VEHICLE_KEY_Y, 42);
            game.world
                .block_entities
                .insert(vehicle, BlockEntity::Chest(vec![Some(stone.clone())]));
            game.lan.as_mut().unwrap().hub.open_container(
                1,
                "minecraft:generic_9x1",
                title,
                vehicle,
                Slots::Block(vehicle),
                vec![None],
                None,
            );
            let menu = game.player_container_menu(player).unwrap();
            assert_eq!(
                game.menu_slot_backing(menu, 0),
                Some(MenuSlotBacking::Entity {
                    entity: EntityRef::new(game.level("minecraft:overworld").unwrap().epoch(), 42),
                    slot: 0
                })
            );
            assert_eq!(
                menu.slot(game, 0),
                Some(Some(stone.try_into_api().unwrap()))
            );

            // Identical coordinates in different dimensions must not alias.
            let gold = ItemStack::new("minecraft:gold_ingot", 2);
            let emerald = ItemStack::new("minecraft:emerald", 3);
            game.world
                .block_entities
                .insert(a, BlockEntity::Chest(vec![Some(gold.clone())]));
            let title = sunrise_core::nbt::Tag::String("dimension".into());
            game.lan.as_mut().unwrap().hub.open_container(
                1,
                "minecraft:generic_9x1",
                title.clone(),
                a,
                Slots::Block(a),
                vec![Some(gold.clone())],
                None,
            );
            let overworld_menu = game.player_container_menu(player).unwrap();
            let overworld = game.level("minecraft:overworld").unwrap();
            assert!(
                game.enter_command_level(super::super::Dim::Nether)
                    .is_some()
            );
            game.world
                .block_entities
                .insert(a, BlockEntity::Chest(vec![Some(emerald.clone())]));
            let mut nether_guest = guest();
            nether_guest.conn = 2;
            nether_guest.uuid = 456;
            nether_guest.entity_id = 2;
            nether_guest.dimension = "minecraft:the_nether".into();
            game.lan.as_mut().unwrap().hub.guests.push(nether_guest);
            game.lan.as_mut().unwrap().hub.open_container(
                2,
                "minecraft:generic_9x1",
                title,
                a,
                Slots::Block(a),
                vec![Some(emerald.clone())],
                None,
            );
            let other_player = game.player(456).unwrap();
            let nether_menu = game.player_container_menu(other_player).unwrap();
            let nether = game.level("minecraft:the_nether").unwrap();
            assert_eq!(
                overworld_menu.slot(game, 0),
                Some(Some(gold.try_into_api().unwrap()))
            );
            assert_eq!(
                game.menu_slot_backing(overworld_menu, 0),
                Some(MenuSlotBacking::BlockEntity {
                    level: overworld,
                    pos: [a.0, a.1, a.2],
                    slot: 0
                })
            );
            assert!(
                game.enter_command_level(super::super::Dim::Overworld)
                    .is_some()
            );
            assert_eq!(
                nether_menu.slot(game, 0),
                Some(Some(emerald.try_into_api().unwrap()))
            );
            assert_eq!(
                game.menu_slot_backing(nether_menu, 0),
                Some(MenuSlotBacking::BlockEntity {
                    level: nether,
                    pos: [a.0, a.1, a.2],
                    slot: 0
                })
            );
        });
    }
}
