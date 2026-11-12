//! Server menus for native mods (docs/modding/src/how-to/native-mods.md, "Server menus"): a vanilla chest menu (`generic_9x1` to `generic_9x6`) a mod opens
//! for a player and fills with items to look at. No click moves anything:
//! each reaches the mod (`native_menu_clicked`), as does the menu's closing
//! (`native_menu_closed`), and the client shows the menu as the server sends
//! it again. A stock client shows it; nothing on the player's side. A trade
//! built on these menus moves the items between the players' `ENTITY_ITEMS`
//! handlers in one transaction, so nothing moves before both sides are settled.
use super::{Game, lan::LanHost};
use horizon_client::{
    items::ItemStack,
    lan::{data::component_nbt, menu::NativeMenuEvent},
    text::Text,
};

/// The inventory slot (0..9 the hotbar, 9..36 the rest) a click on `slot`
/// of a native menu of `size` slots landed on, when it is the player's
/// inventory below the menu (`ChestMenu`: the menu's slots, then the
/// inventory's 27, then the hotbar's 9).
#[allow(dead_code)]
pub(crate) fn native_menu_inventory_slot(size: usize, slot: i32) -> Option<usize> {
    let n = usize::try_from(slot).ok()?.checked_sub(size)?;
    match n {
        0..27 => Some(n + 9),
        27..36 => Some(n - 27),
        _ => None,
    }
}

/// Adds every stack of `gets` to the 36 slots of `inventory`, onto matching
/// stacks first, then into empty slots, hotbar first; false when one doesn't
/// fit whole. `max` is an item's stack size.
pub(super) fn fit(
    inventory: &mut [Option<ItemStack>],
    gets: &[ItemStack],
    max: &impl Fn(&ItemStack) -> u32,
) -> bool {
    for stack in gets {
        let mut left = stack.count;
        let max = max(stack).max(1);
        for slot in inventory.iter_mut().take(36).flatten() {
            if left > 0 && slot.same_item_same_components(stack) && slot.count < max {
                let n = (max - slot.count).min(left);
                slot.count += n;
                left -= n;
            }
        }
        for slot in inventory.iter_mut().take(36) {
            if left > 0 && slot.is_none() {
                let n = max.min(left);
                *slot = Some(ItemStack {
                    count: n,
                    ..stack.clone()
                });
                left -= n;
            }
        }
        if left > 0 {
            return false;
        }
    }
    true
}

/// Server menus, for server mods. The players are named by UUID;
/// none of these runs while a player's state is swapped in (inside
/// `native_run_mod_command`, the menus can be opened and filled).
#[allow(dead_code)]
impl Game<'_> {
    fn native_conn(&self, uuid: u128) -> Option<u32> {
        let host = self.lan.as_ref()?;
        let g = host
            .hub
            .guests
            .iter()
            .find(|g| g.placed && g.participates_online() && g.uuid == uuid)?;
        Some(g.conn)
    }
    /// Opens a chest menu of `rows` rows (1 to 6) titled `title` for the
    /// player, showing `items` (by slot, padded with empty ones), in place of
    /// the menu they had open: its window id.
    pub(super) fn open_menu(
        &mut self,
        uuid: u128,
        rows: u8,
        title: Text,
        items: Vec<Option<ItemStack>>,
    ) -> Option<i32> {
        let conn = self.native_conn(uuid)?;
        // ServerPlayer.openMenu closes the menu that was open.
        self.guest_close_all(conn);
        self.lan.as_mut()?.hub.open_native_menu(
            conn,
            rows,
            component_nbt(&title.component()),
            items,
        )
    }
    /// Replaces what the player's open native menu `window` shows, without reopening it.
    /// False when that window isn't their open native menu.
    pub(super) fn menu_set_items(
        &mut self,
        uuid: u128,
        window: i32,
        items: Vec<Option<ItemStack>>,
    ) -> bool {
        let Some(conn) = self.native_conn(uuid) else {
            return false;
        };
        self.lan
            .as_mut()
            .is_some_and(|host| host.hub.set_native_menu_items(conn, window, items))
    }
    /// Closes the player's native menu (a `native_menu_closed` follows).
    pub(super) fn close_menu(&mut self, uuid: u128) {
        if let Some(conn) = self.native_conn(uuid)
            && let Some(host) = self.lan.as_mut()
            && host
                .hub
                .guest(conn)
                .and_then(|g| g.menu.as_ref())
                .is_some_and(|m| m.slots == horizon_client::lan::menu::Slots::Native)
        {
            host.hub.close_container(conn);
        }
    }
}

/// A menu a player has open, as the mods last heard it
/// (`native_player_container_open`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct HeardMenu {
    /// The window id the player's client knows it by.
    window: i32,
    /// The menu type's name; a mount's inventory has none in vanilla.
    menu: String,
    /// The block the menu is of.
    pos: Option<[i32; 3]>,
}

/// What a mount's inventory is called, as vanilla's `HorseInventoryMenu` has
/// no menu type.
const MOUNT_MENU: &str = "horizon:mount_inventory";

/// The menu `conn` has open, whichever way the game keeps it: a block's
/// container, a trade, a crafting table, a workstation, a lectern or a
/// mount's inventory. Not the player's own inventory menu.
pub(super) fn open_menu_of(host: &LanHost, conn: u32) -> Option<(i32, &str, Option<[i32; 3]>)> {
    let g = host.hub.guest(conn)?;
    let play = host.play.get(&conn);
    if let Some(m) = &g.menu {
        let menu = host.hub.registry_name("minecraft:menu", m.kind)?;
        // A vehicle's menu is keyed by a stand-in, not a block.
        let pos = m
            .pos
            .filter(|p| p.1 != horizon_client::lan::menu::VEHICLE_KEY_Y)
            .map(|p| [p.0, p.1, p.2]);
        return Some((m.window, menu, pos));
    }
    let play = play?;
    let (window, menu, pos) = if let Some(gm) = &play.merchant {
        (gm.window, "minecraft:merchant", None)
    } else if let Some((window, _, _, pos)) = play.crafting.table.as_ref() {
        (*window, "minecraft:crafting", Some(*pos))
    } else if let Some(st) = play.station.as_ref() {
        (
            st.window,
            super::lan_stations::menu_of(st.kind)?.0,
            Some(st.pos),
        )
    } else if let Some(l) = &play.lectern {
        (
            l.window,
            "minecraft:lectern",
            Some([l.key.0, l.key.1, l.key.2]),
        )
    } else if let Some(m) = &play.mount_window {
        (m.window, MOUNT_MENU, None)
    } else {
        return None;
    };
    Some((window, menu, pos))
}

/// Where the game calls the menu hook points.
impl Game<'_> {
    /// The mods hear the menus the players here opened or closed since the
    /// last tick (`PlayerContainerEvent`), in that order for a menu another
    /// replaced: what each player has open is compared with what the mods last
    /// heard. A menu opened and closed again inside one tick is not heard.
    pub(super) fn tick_player_menus(&mut self) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let conns: Vec<u32> = host
            .hub
            .guests
            .iter()
            .filter(|g| host.hub.here(g))
            .map(|g| g.conn)
            .collect();
        for conn in conns {
            let Some(host) = self.lan.as_ref() else {
                return;
            };
            // `AbstractContainerScreen.tick` closes the container once the player is no longer
            // alive, so a death closes the menu now (the lid falls, the mods hear Close), not at
            // the respawn.
            let dead = host.hub.guest(conn).is_some_and(|g| g.dead);
            if dead && host.hub.guest(conn).is_some_and(|g| g.menu.is_some()) {
                self.guest_container_closed(conn);
                if let Some(host) = self.lan.as_mut() {
                    host.hub.close_container(conn);
                }
            }
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            // Compared as borrowed: nothing is built each tick while it stays open.
            let now = if dead { None } else { open_menu_of(host, conn) };
            let heard = host.play.get(&conn).and_then(|p| p.menu_heard.as_ref());
            if now == heard.map(|h| (h.window, h.menu.as_str(), h.pos)) {
                continue;
            }
            let now = now.map(|(window, menu, pos)| HeardMenu {
                window,
                menu: menu.to_owned(),
                pos,
            });
            let heard = host.play.get(&conn).and_then(|p| p.menu_heard.clone());
            // Remembered first: a hook can run commands that close menus.
            if let Some(p) = host.play.get_mut(&conn) {
                p.menu_heard.clone_from(&now);
            }
            if let Some(old) = heard {
                self.player_menu_closed(conn, old);
            }
            if let Some(new) = now {
                self.player_menu_opened(conn, new);
            }
        }
    }

    /// `conn`'s menu closes with the guest in the level in place
    /// (`ServerPlayer.doCloseContainer`): the mods hear it, if they heard it
    /// open.
    pub(super) fn player_menu_closing(&mut self, conn: u32) {
        let heard = self
            .lan
            .as_mut()
            .and_then(|host| host.play.get_mut(&conn))
            .and_then(|p| p.menu_heard.take());
        if let Some(old) = heard {
            self.player_menu_closed(conn, old);
        }
    }

    fn player_menu_opened(&mut self, conn: u32, menu: HeardMenu) {
        let Some((uuid, name)) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| (g.uuid, g.name.clone()))
        else {
            return;
        };
        let dimension = self.dim.id();
        self.player_hook(conn, move |g| {
            g.native_player_container_open(uuid, &name, dimension, &menu.menu, menu.pos);
        });
    }

    fn player_menu_closed(&mut self, conn: u32, menu: HeardMenu) {
        let Some((uuid, name)) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| (g.uuid, g.name.clone()))
        else {
            return;
        };
        let dimension = self.dim.id();
        self.player_hook(conn, move |g| {
            g.native_player_container_close(uuid, &name, dimension, &menu.menu, menu.pos);
        });
    }

    /// Runs a player hook with the guest's state in place, or as it is when
    /// the guest has no body to swap in.
    fn player_hook(&mut self, conn: u32, f: impl FnOnce(&mut Self)) {
        let mut f = Some(f);
        if self.lan_acting != Some(conn) {
            self.as_guest(conn, |g| {
                if let Some(f) = f.take() {
                    f(g);
                }
            });
        }
        if let Some(f) = f {
            f(self);
        }
    }

    /// The clicks on and closings of native menus the hub collected, each
    /// told to the mods with the hub in `self.lan`, no player's state
    /// swapped in.
    pub(super) fn run_native_menu_events(&mut self) {
        loop {
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            if host.hub.native_menu_events.is_empty() {
                return;
            }
            match host.hub.native_menu_events.remove(0) {
                NativeMenuEvent::Click {
                    uuid,
                    window,
                    slot,
                    button,
                    mode,
                } => self.native_menu_clicked(uuid, window, slot, button, mode),
                NativeMenuEvent::Closed { uuid, window } => self.native_menu_closed(uuid, window),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_clicks_map_to_the_inventory_below() {
        // A 6-row chest: 54 slots, then the inventory's 27, then the hotbar.
        assert_eq!(native_menu_inventory_slot(54, 3), None);
        assert_eq!(native_menu_inventory_slot(54, 54), Some(9));
        assert_eq!(native_menu_inventory_slot(54, 80), Some(35));
        assert_eq!(native_menu_inventory_slot(54, 81), Some(0));
        assert_eq!(native_menu_inventory_slot(54, 89), Some(8));
        assert_eq!(native_menu_inventory_slot(54, 90), None);
        assert_eq!(native_menu_inventory_slot(54, -999), None);
        assert_eq!(native_menu_inventory_slot(9, 9), Some(9));
    }
}
