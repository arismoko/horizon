//! Mod menu slots backed by storage that is not a block entity (arismoko/horizon#3385):
//! NeoForge 26.3's `StackCopySlot` and `ResourceHandlerSlot`. A mod opens its menu with
//! `player_open_mod_menu` (#3384) and backs its container slots with
//! `player_back_mod_menu_slots`. The guest's `native_menu` then mirrors those places, so the
//! client sees ordinary synced slots, and a pickup or shift-click on the menu goes through the
//! places' own handlers in one transaction. Nothing runs per tick: the mirror is refreshed after
//! each click on the menu and at each change notice (an `ItemPlace` write to the backing
//! player's places, queued as `NativeMenuEvent::PlacesChanged` and taken after the write), and
//! only slots whose stack changed are written. A menu no mod backed never reaches this module's
//! work: `GuestMenus::slot_places` stays `None` and every path below returns first.
use super::native_menus::native_menu_inventory_slot;
use crate::game::ServerGame;
use mod_api::__private::game::*;
use mod_api::{
    Hand, ItemPlace, ItemResource, MenuContainer, MenuRef, MenuSlotBackRefusal, MenuSlotPlace,
    ServerHost,
};
use sunrise_gameplay_common::game::api_values::TryIntoEngine;
use sunrise_gameplay_common::game::native_menu_values::MenuSlotPlaces;
use sunrise_items::items::ItemStack;
use sunrise_sim::lan::{
    guest::Guest,
    menu::{NativeMenuEvent, Slots},
};
use sunrise_sim::menu::{SlotContext, SlotRules, apply_menu_slot_rules, item_max_stack};

/// A held container's identity: the stack without the contents its `ITEM_ITEMS` handler
/// writes (`minecraft:container`, `minecraft:bundle_contents`), so writes through the menu
/// keep it and another stack put in its place does not.
fn held_identity(stack: &ItemStack) -> ItemStack {
    let mut stack = stack.clone();
    if let Some(data) = stack.extra.as_deref_mut() {
        data.container.clear();
        data.bundle_contents.clear();
        data.set_other("minecraft:bundle_contents", None);
    }
    stack.tidy();
    stack
}

impl ServerGame<'_> {
    /// The inventory slot holding `hand`'s stack (the selected hotbar slot, or the offhand's 40) and
    /// that stack, of `guest`: read from the game in their own turn, when their record holds the
    /// outer player's.
    pub(in crate::game::server_side) fn held_by<'a>(
        &'a self,
        guest: &'a Guest,
        hand: Hand,
    ) -> (usize, Option<&'a ItemStack>) {
        let (selected, inventory, offhand) = match self.server.lan_acting == Some(guest.conn) {
            true => (
                self.common.selected,
                &self.common.inventory,
                &self.common.offhand,
            ),
            false => (guest.selected, &guest.inventory, &guest.offhand),
        };
        match hand {
            Hand::Off => (40, offhand.as_ref()),
            _ => (selected, inventory.get(selected).and_then(Option::as_ref)),
        }
    }

    /// `menu_back_slots`: backs `menu`'s first container slots with `places`. An
    /// empty list takes the backing away; a slot no longer backed is emptied, so the copy it
    /// showed never becomes the menu's own storage. In the player's own turn (an item's use
    /// that opened the menu), the record holds the outer player's inventory, so mirroring
    /// and resending wait until after the turn.
    pub fn menu_back_slots(
        &mut self,
        menu: MenuRef,
        places: Vec<MenuSlotPlace>,
    ) -> Result<(), MenuSlotBackRefusal> {
        let player = menu.fields().player;
        let window = menu.fields().window;
        let uuid = player.uuid();
        let guest = self
            .native_menu_guest(menu)
            .ok_or(MenuSlotBackRefusal::NoPlayerOrWindow)?;
        let Some(open) = guest.menu.as_ref() else {
            return Err(MenuSlotBackRefusal::UnsupportedMenu);
        };
        // Only a blockless mod menu's slots are the game's to show (`Slots::Native`); a block's
        // or an entity's menu reads its own storage, which a backing would not replace.
        if open.slots != Slots::Native || open.owned_slots().is_some() {
            return Err(MenuSlotBackRefusal::UnsupportedMenu);
        }
        // The stacks the held places are bound to, as they are now.
        let mut held = [None, None];
        for place in &places {
            if let MenuSlotPlace::HeldItem { hand, .. } = *place {
                let (index, stack) = self.held_by(guest, hand);
                held[usize::from(hand == Hand::Off)] = stack.map(|s| (index, held_identity(s)));
            }
        }
        let selected = self.held_by(guest, Hand::Main).0;
        let conn = guest.conn;
        let size = open.size;
        let acting = self.server.lan_acting == Some(conn);
        if places.len() > size {
            return Err(MenuSlotBackRefusal::TooManyPlaces);
        }
        let host: &mut dyn ServerHost = self;
        if let Some(index) = places
            .iter()
            .position(|place| !place.resolves(host, player))
        {
            return Err(MenuSlotBackRefusal::PlaceUnresolved { index });
        }
        let locked = places
            .iter()
            .any(|place| {
                matches!(
                    place,
                    MenuSlotPlace::HeldItem {
                        hand: Hand::Main,
                        ..
                    }
                )
            })
            .then_some(selected);
        let transactional = !self.common.transactions.is_empty();
        if transactional {
            self.journal_menus(player);
            self.journal_menu_click(conn);
        }
        let Some(lan) = self.server.lan.as_mut() else {
            return Err(MenuSlotBackRefusal::NoPlayerOrWindow);
        };
        let count = places.len();
        let entry = &mut lan.menus.entry(conn).or_default().slot_places;
        // The slots this window's old backing mirrored that the new one does not show.
        let before = entry
            .as_ref()
            .filter(|old| old.window == window)
            .map_or(0, |old| old.places.len());
        *entry = (count > 0).then(|| MenuSlotPlaces {
            window,
            places,
            locked,
            held,
            // A notice belongs to the window, not the backing that happened to queue it.
            noticed: lan.hub.native_menu_events.iter().any(|event| {
                matches!(event, NativeMenuEvent::PlacesChanged { uuid: owner, window: queued }
                    if *owner == uuid && *queued == window)
            }),
        });
        if let Some(index) = lan.hub.guests.connection_index(conn)
            && let Some(shown) = lan.hub.guests.native_menu_mut(index)
        {
            for slot in shown.iter_mut().take(before).skip(count) {
                *slot = None;
            }
        }
        if transactional && !acting {
            self.after_commit(
                sunrise_gameplay_common::game::transactions::After::BackedMenuClick(
                    conn, uuid, menu, window,
                ),
            );
            return Ok(());
        }
        if acting {
            // Slots cleared above reach the client through the container sync after the turn.
            self.after_commit(
                sunrise_gameplay_common::game::transactions::After::MenuPlacesNotice(
                    conn, uuid, window,
                ),
            );
            return Ok(());
        }
        if count > 0 {
            self.refresh_menu_places(conn, menu);
        }
        if before > count
            && let Some(lan) = self.server.lan.as_mut()
        {
            lan.hub.resend_menu(conn, window, &self.common.world);
        }
        Ok(())
    }

    /// What container slot `n` of `conn`'s menu `window` shows, when a mod backed it.
    pub(in crate::game) fn native_menu_place(
        &self,
        conn: u32,
        window: i32,
        n: usize,
    ) -> Option<MenuSlotPlace> {
        let backed = self.active_menu_places(conn)?;
        (backed.window == window).then_some(())?;
        backed.places.get(n).copied()
    }

    /// The backing of `conn`'s menu: its window, how many slots and the locked hotbar slot.
    pub fn menu_places_of(&self, conn: u32) -> Option<(i32, usize, Option<usize>)> {
        let backed = self.active_menu_places(conn)?;
        Some((backed.window, backed.places.len(), backed.locked))
    }

    fn active_menu_places(&self, conn: u32) -> Option<&MenuSlotPlaces> {
        let lan = self.server.lan.as_ref()?;
        let backed = lan.menus.get(&conn)?.slot_places.as_ref()?;
        let open = lan.hub.guest(conn)?.menu.as_ref()?;
        (open.window == backed.window && open.slots == Slots::Native).then_some(backed)
    }

    pub(in crate::game::server_side) fn forget_menu_places(&mut self, conn: u32) {
        if let Some(menus) = self
            .server
            .lan
            .as_mut()
            .and_then(|lan| lan.menus.get_mut(&conn))
        {
            menus.slot_places = None;
        }
    }

    /// Whether the stacks `conn`'s held places were bound to are still where they were: the same
    /// inventory slot (the same hotbar slot selected) holding the same stack, its contents aside.
    pub(super) fn held_places_bound(&self, conn: u32, player: mod_api::PlayerRef) -> bool {
        let Some(backed) = self
            .server
            .lan
            .as_ref()
            .and_then(|lan| lan.menus.get(&conn)?.slot_places.as_ref())
        else {
            return true;
        };
        if backed.held.iter().all(Option::is_none) {
            return true;
        }
        let Some(guest) = self.online_player_record(player) else {
            return false;
        };
        [Hand::Main, Hand::Off]
            .into_iter()
            .zip(&backed.held)
            .all(|(hand, bound)| {
                let Some((index, identity)) = bound else {
                    return true;
                };
                let (now, stack) = self.held_by(guest, hand);
                now == *index && stack.is_some_and(|stack| held_identity(stack) == *identity)
            })
    }

    /// Copies each place's stack into the guest's `native_menu` where it changed, which the next
    /// container sync sends; closes the menu, as a gone block entity's, once a place stops
    /// resolving or a held place's stack is no longer the bound one. False when it closed or was
    /// not open.
    fn refresh_menu_places(&mut self, conn: u32, menu: MenuRef) -> bool {
        let Some((window, count, _)) = self.menu_places_of(conn) else {
            return false;
        };
        let player = menu.fields().player;
        let mut open = self.held_places_bound(conn, player);
        for n in 0..count {
            let Some(place) = self.native_menu_place(conn, window, n) else {
                break;
            };
            let host: &mut dyn ServerHost = self;
            if !open || !place.resolves(host, player) {
                open = false;
                break;
            }
            let stack = ItemPlace::MenuSlot { menu, slot: n }
                .with_stack(host, |stack| stack.project())
                .flatten()
                .and_then(|stack| stack.try_into_engine(self.items()).ok());
            let Some(lan) = self.server.lan.as_mut() else {
                return false;
            };
            let guests = &mut lan.hub.guests;
            let Some(index) = guests.connection_index(conn) else {
                return false;
            };
            if let Some(shown) = guests.native_menu_mut(index).and_then(|m| m.get_mut(n))
                && *shown != stack
            {
                *shown = stack;
            }
        }
        if !open {
            self.forget_menu_places(conn);
            self.request_menu_close(conn, window, menu.fields().generation);
        }
        open
    }

    /// The storage-change notice: an `ItemPlace` write to `place`, if it is a place of a player
    /// whose open menu a mod backed (their hand or inventory, a container held there, their
    /// menu's slots), queues one look at that menu's places for after the write
    /// (`NativeMenuEvent::PlacesChanged`). Any other write, and every write while no menu is
    /// backed, returns at the first checks.
    pub fn notice_menu_place_write(&mut self, place: &ItemPlace) {
        let mut root = place;
        while let ItemPlace::Nested { parent, .. } = root {
            root = &**parent;
        }
        let uuid = match root {
            ItemPlace::PlayerHand { player, .. } | ItemPlace::PlayerSlot { player, .. } => *player,
            ItemPlace::MenuSlot { menu, .. } => menu.fields().player.uuid(),
            _ => return,
        };
        if self
            .server
            .lan
            .as_ref()
            .is_none_or(|lan| lan.menus.values().all(|m| m.slot_places.is_none()))
        {
            return;
        }
        if let Some(conn) = self.native_conn(uuid)
            && let Some(window) = self.menu_places_of(conn).map(|(window, ..)| window)
        {
            self.after_commit(
                sunrise_gameplay_common::game::transactions::After::MenuPlacesNotice(
                    conn, uuid, window,
                ),
            );
        }
    }

    /// Queues one look at the places of `conn`'s backed menu (`NativeMenuEvent::PlacesChanged`),
    /// unless one is already waiting to be taken.
    pub(in crate::game) fn queue_menu_places_notice(&mut self, conn: u32, uuid: u128, window: i32) {
        let Some(lan) = self.server.lan.as_mut() else {
            return;
        };
        let Some(backed) = lan
            .menus
            .get_mut(&conn)
            .and_then(|m| m.slot_places.as_mut())
            .filter(|backed| backed.window == window && !backed.noticed)
        else {
            return;
        };
        backed.noticed = true;
        lan.hub
            .native_menu_events
            .push(NativeMenuEvent::PlacesChanged { uuid, window });
    }

    /// A change notice taken (`NativeMenuEvent::PlacesChanged`): the backed menu's places looked
    /// at again, or forgotten once that menu is no longer open.
    pub fn menu_places_changed(&mut self, uuid: u128, window: i32) {
        let Some(conn) = self.native_conn(uuid) else {
            return;
        };
        let Some(backed) = self
            .server
            .lan
            .as_mut()
            .and_then(|lan| lan.menus.get_mut(&conn)?.slot_places.as_mut())
            .filter(|backed| backed.window == window)
        else {
            return;
        };
        backed.noticed = false;
        match self.native_menu_ref(uuid, window) {
            Some(menu) => {
                self.refresh_menu_places(conn, menu);
            }
            None => self.forget_menu_places(conn),
        }
    }

    /// The menu `window` closed (`NativeMenuEvent::Closed`): its places end with it. Its
    /// mirrored slots went with the menu (`native_menu` is cleared on close).
    pub(in crate::game::server_side) fn menu_places_closed(&mut self, uuid: u128, window: i32) {
        if let Some(conn) = self.native_conn(uuid)
            && self
                .server
                .lan
                .as_ref()
                .and_then(|lan| lan.menus.get(&conn)?.slot_places.as_ref())
                .is_some_and(|backed| backed.window == window)
        {
            self.forget_menu_places(conn);
        }
    }

    /// The inventory slot of a held backpack takes no click.
    pub(super) fn backed_menu_slot_enabled(&self, guest: &Guest, slot: usize) -> bool {
        let Some(open) = &guest.menu else {
            return true;
        };
        let locked = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.menus.get(&guest.conn)?.slot_places.as_ref())
            .filter(|backed| backed.window == open.window && open.slots == Slots::Native)
            .and_then(|backed| backed.locked);
        !locked.is_some_and(|locked| {
            i32::try_from(slot)
                .ok()
                .and_then(|slot| native_menu_inventory_slot(open.size, slot))
                == Some(locked)
        })
    }

    /// What the menu's own slot rules (`menu_slot_rules`, the locked held-backpack slot) say of a
    /// backed slot: whether it may take `amount` of `resource`, whether it may give its stack up,
    /// and how many it holds. Click admission and the slot probes both ask this, so a probe
    /// answers what a click would do.
    pub(super) fn place_menu_rules(
        &self,
        conn: u32,
        size: usize,
        locked: Option<usize>,
        slot: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> (bool, bool, u32) {
        if locked.is_some() && native_menu_inventory_slot(size, slot as i32) == locked {
            return (false, false, 0);
        }
        let Some(guest) = self.server.lan.as_ref().and_then(|lan| lan.hub.guest(conn)) else {
            return (false, false, 0);
        };
        let Some(open) = guest.menu.as_ref() else {
            return (false, false, 0);
        };
        if !self.listens(&open.registry_id) {
            return (true, true, u32::MAX);
        }
        let Some(stack) = resource
            .to_stack(amount)
            .and_then(|stack| stack.try_into_engine(self.items()).ok())
        else {
            return (true, true, u32::MAX);
        };
        apply_menu_slot_rules(
            Some(self),
            &open.registry_id,
            native_menu_inventory_slot(size, slot as i32).unwrap_or(slot),
            &stack,
            SlotContext::blockless(
                guest,
                open.pos.map(|(x, y, z)| [x, y, z]),
                if slot < size {
                    MenuContainer::Main
                } else {
                    MenuContainer::Player
                },
            ),
            (true, true, u32::MAX),
            item_max_stack(self.items(), &stack),
        )
    }

    /// A click on a backed menu (`AbstractContainerMenu.clicked`'s `PICKUP` and `QUICK_MOVE`),
    /// done through the places' handlers in one transaction. `None` lets ordinary display-menu
    /// routing run, for a menu no mod backed or a click this backing does not handle. Before any
    /// click on a backed menu a held place's stack must still be the bound one; else the menu
    /// closes and the click does nothing.
    pub fn place_menu_click(
        &mut self,
        uuid: u128,
        window: i32,
        slot: i32,
        button: i32,
        mode: i32,
    ) -> Option<bool> {
        use sunrise_gameplay_common::game::lan_menu_data::ClickReply;
        let conn = self.native_conn(uuid)?;
        let (backed_window, count, locked) = self.menu_places_of(conn)?;
        if backed_window != window {
            return None;
        }
        let menu = self.native_menu_ref(uuid, window)?;
        if !self.held_places_bound(conn, menu.fields().player) {
            self.reply_menu_click(conn, ClickReply::Close(window));
            return Some(true);
        }
        let at = usize::try_from(slot).ok()?;
        let guest = self.native_menu_guest(menu)?;
        let open = guest.menu.as_ref()?;
        let (size, total) = (open.size, open.slot_count());
        let pickup = mode == 0 && matches!(button, 0 | 1);
        if !(pickup || mode == 1) || (count..size).contains(&at) || at >= total {
            return None;
        }
        let inventory = native_menu_inventory_slot(size, slot);
        // The held backpack's own slot takes no click, so it can't go into itself.
        let applied = if at < count
            && (ItemPlace::MenuSlot { menu, slot: at }).amount(self) > 0
            && menu.probe_slot_gives(self, at) != Ok(Some(true))
        {
            true
        } else if inventory.is_none() || inventory != locked {
            let accepts =
                |game: &crate::game::ServerGame<'_>, slot, resource: &ItemResource, amount| {
                    game.place_menu_rules(conn, size, locked, slot, resource, amount)
                };
            if pickup {
                click_pickup(self, menu, at, button == 1, accepts)
            } else {
                click_quick_move(self, menu, at, size, total - size, count, accepts)
            }
        } else {
            true
        };
        if applied {
            self.after_commit(
                sunrise_gameplay_common::game::transactions::After::BackedMenuClick(
                    conn, uuid, menu, window,
                ),
            );
        } else {
            self.reply_menu_click(conn, ClickReply::Resend(window));
        }
        Some(applied)
    }

    pub(in crate::game) fn publish_backed_menu_click(
        &mut self,
        conn: u32,
        uuid: u128,
        menu: MenuRef,
        window: i32,
    ) {
        if self.native_menu_ref(uuid, window) == Some(menu)
            && (self.menu_places_of(conn).is_none() || self.refresh_menu_places(conn, menu))
        {
            self.reply_menu_click(
                conn,
                sunrise_gameplay_common::game::lan_menu_data::ClickReply::Resend(window),
            );
        }
    }
}

/// `PICKUP` on menu slot `at`: an empty cursor takes all (or half, `right`) of the slot; a
/// full one puts all (or one) in, or swaps with a slot of another item. All or nothing.
fn click_pickup(
    game: &mut crate::game::ServerGame<'_>,
    menu: MenuRef,
    slot: usize,
    right: bool,
    accepts: impl Fn(&crate::game::ServerGame<'_>, usize, &ItemResource, u32) -> (bool, bool, u32),
) -> bool {
    let cursor = menu.cursor();
    let at = ItemPlace::MenuSlot { menu, slot };
    game.storage_transaction(|host, tx| {
        let here = at.resource(host);
        let have = at.amount(host);
        if let Some(here) = &here
            && !accepts(host, slot, here, have).1
        {
            return Ok(());
        }
        let Some(held) = cursor.resource(host) else {
            let Some(here) = here else {
                return Ok(());
            };
            let want = if right { have.div_ceil(2) } else { have };
            let out = at.extract(host, tx, &here, want).map_err(|_| ())?;
            return match cursor.insert(host, tx, &here, out) == Ok(out) {
                true => Ok(()),
                false => Err(()),
            };
        };
        let carried = cursor.amount(host);
        let (place, _, max) = accepts(host, slot, &held, carried);
        if !place {
            return Ok(());
        }
        let room = if here.as_ref() == Some(&held) {
            max.saturating_sub(have)
        } else {
            max
        };
        let put = at
            .insert(host, tx, &held, (if right { 1 } else { carried }).min(room))
            .map_err(|_| ())?;
        if put > 0 {
            return match cursor.extract(host, tx, &held, put) == Ok(put) {
                true => Ok(()),
                false => Err(()),
            };
        }
        let Some(here) = here.filter(|here| *here != held && carried <= max) else {
            return Ok(());
        };
        let swapped = at.extract(host, tx, &here, have) == Ok(have)
            && cursor.extract(host, tx, &held, carried) == Ok(carried)
            && at.insert(host, tx, &held, carried) == Ok(carried)
            && cursor.insert(host, tx, &here, have) == Ok(have);
        match swapped {
            true => Ok(()),
            false => Err(()),
        }
    })
    .is_ok()
}

/// `QUICK_MOVE` on menu slot `at` of a menu of `size` container slots, `count` of them backed
/// (`ChestMenu.quickMoveStack`): a container slot's stack goes to the inventory, hotbar end
/// first; an inventory slot's into the backed slots in order, as allowed by the menu's slot rules.
/// Onto matching stacks first, then into empty slots (`moveItemStackTo`), all or
/// nothing.
fn click_quick_move(
    game: &mut crate::game::ServerGame<'_>,
    menu: MenuRef,
    at: usize,
    size: usize,
    player_slots: usize,
    count: usize,
    accepts: impl Fn(&crate::game::ServerGame<'_>, usize, &ItemResource, u32) -> (bool, bool, u32),
) -> bool {
    let from = ItemPlace::MenuSlot { menu, slot: at };
    let Some(resource) = from.resource(game) else {
        return true;
    };
    let amount = from.amount(game);
    if !accepts(game, at, &resource, amount).1 {
        return true;
    }
    let (first, len, reverse) = match at < size {
        true => (size, player_slots, true),
        false => (0, count, false),
    };
    game.storage_transaction(|host, tx| {
        let mut left = amount;
        for merge in [true, false] {
            for i in 0..len {
                if left == 0 {
                    return Ok(());
                }
                let slot = if reverse {
                    first + len - 1 - i
                } else {
                    first + i
                };
                let to = ItemPlace::MenuSlot { menu, slot };
                match to.resource(host) {
                    Some(here) if merge && here == resource => {}
                    None if !merge => {}
                    _ => continue,
                }
                let (place, _, max) = accepts(host, slot, &resource, left);
                if !place {
                    continue;
                }
                let room = max.saturating_sub(to.amount(host));
                let moved = to
                    .insert(host, tx, &resource, left.min(room))
                    .map_err(|_| ())?;
                if moved > 0 {
                    if from.extract(host, tx, &resource, moved) != Ok(moved) {
                        return Err(());
                    }
                    left -= moved;
                }
            }
        }
        Ok(())
    })
    .is_ok()
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
