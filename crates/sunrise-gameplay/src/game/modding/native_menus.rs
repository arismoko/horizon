//! Blockless server menus for native mods: vanilla chest displays or declared mod menus.
//! Owned slots run ordinary clicks; display slots move nothing by default. Menu parts can
//! replace a click, and the owning menu part receives any detached owned contents.
use super::{lan::LanHost, lan_menus::GuestMenus};
use mod_api::__private::game::*;
use mod_api::{MenuRef, MenuReplace, ModMenuOpen};
use sunrise_core::nbt::Tag;
use sunrise_game_data::text::Text;
use sunrise_gameplay_common::game::api_values::TryIntoEngine;
use sunrise_gameplay_common::game::native_menu_data::{ContainerPos, HeardMenu};
use sunrise_gameplay_common::game::native_menu_ids::MOUNT_MENU;
use sunrise_items::items::ItemStack;
use sunrise_level::containers::ModMenuType;
use sunrise_sim::lan::{
    data::component_nbt,
    menu::{Menu, NativeMenuEvent, Slots},
};

/// The inventory slot (0..9 the hotbar, 9..36 the rest) a click on `slot`
/// of a native menu of `size` slots landed on, when it is the player's
/// inventory below the menu (`ChestMenu`: the menu's slots, then the
/// inventory's 27, then the hotbar's 9).
#[allow(dead_code)]
pub(in crate::game::server_side) fn native_menu_inventory_slot(
    size: usize,
    slot: i32,
) -> Option<usize> {
    let n = usize::try_from(slot).ok()?.checked_sub(size)?;
    match n {
        0..27 => Some(n + 9),
        27..36 => Some(n - 27),
        _ => None,
    }
}

/// An open that must not replace the current menu or return its cursor until commit.
pub(in crate::game) enum PendingMenuOpen {
    Native(NativeMenuOpen),
    ModMenu(PendingModMenuOpen),
    Block(mod_api::PlayerRef, mod_api::BlockEntityRef),
    Ender(mod_api::PlayerRef, Text),
}

pub(in crate::game) struct PendingModMenuOpen {
    player: mod_api::PlayerRef,
    id: String,
    kind: &'static ModMenuType,
    title: Tag,
    extra: Vec<u8>,
    data: Vec<i32>,
    pos: Option<[i32; 3]>,
    opening_hand: Option<mod_api::Hand>,
    replace: MenuReplace,
}

pub(in crate::game) struct NativeMenuOpen {
    menu: String,
    title: Tag,
    items: Vec<Option<ItemStack>>,
    replace: MenuReplace,
    pos: Option<(i32, i32, i32)>,
    extra: Option<Vec<u8>>,
    content: Option<ModMenuData>,
}

struct ModMenuData {
    kind: &'static ModMenuType,
    data: Vec<i32>,
    opening_item: Option<(mod_api::Hand, Option<ItemStack>)>,
    opener_block: Option<Box<str>>,
}

/// Blockless server menus, resolved through the player and current menu handles.
#[allow(dead_code)]
impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn native_conn(&self, uuid: u128) -> Option<u32> {
        let host = self.server.lan.as_ref()?;
        let g = host
            .hub
            .guests
            .iter()
            .find(|g| g.placed && g.participates_online() && g.uuid == uuid)?;
        Some(g.conn)
    }

    /// Opens a chest menu of `rows` rows (1 to 6) titled `title` for the
    /// player, showing `items` (by slot, padded with empty ones), in place of
    /// the menu they had open: its handle, with a new window id each time. The old
    /// menu closes as a close does, its cursor stack going back to the
    /// inventory, and `replace` says whether the client is first sent a close.
    pub(in crate::game) fn player_open_chest_menu(
        &mut self,
        player: mod_api::PlayerRef,
        rows: u8,
        title: Text,
        mut items: Vec<Option<ItemStack>>,
        replace: MenuReplace,
    ) -> Result<MenuRef, mod_api::OpenMenuError> {
        use mod_api::OpenMenuError;
        if !self.common.is_logical_server() {
            return Err(OpenMenuError::PlayerUnavailable);
        }
        let title = component_nbt(&title.component());
        sunrise_protocol::buf::check_network_nbt(&title)
            .map_err(|_| OpenMenuError::TitleNotEncodable)?;
        let guest = self
            .online_player_record(player)
            .filter(|g| !g.dead && !g.disconnect_pending)
            .ok_or(OpenMenuError::PlayerUnavailable)?;
        let conn = guest.conn;
        if self.menu_click_in_progress(conn) {
            return Err(OpenMenuError::ClickInProgress);
        }
        let menu = format!("minecraft:generic_9x{rows}");
        self.server
            .lan
            .as_ref()
            .and_then(|host| host.hub.registry_id("minecraft:menu", &menu))
            .ok_or(OpenMenuError::PlayerUnavailable)?;
        items.resize(usize::from(rows) * 9, None);
        let (window, generation) = self
            .request_native_menu_open(
                conn,
                NativeMenuOpen {
                    menu,
                    title,
                    items,
                    replace,
                    pos: None,
                    extra: None,
                    content: None,
                },
            )
            .ok_or(OpenMenuError::PlayerUnavailable)?;
        Ok(menu_ref(player, window, generation))
    }

    /// Returns the vanilla window id (`1..=100`) and generation when installed. A transaction
    /// or an older publication stages the request with a negative identity and generation zero,
    /// without changing the current menu or allocating a wire id.
    fn request_native_menu_open(&mut self, conn: u32, open: NativeMenuOpen) -> Option<(i32, u64)> {
        if self.menu_open_waits(conn) {
            return Some((self.defer_menu_open(conn, PendingMenuOpen::Native(open)), 0));
        }
        self.in_menu_owner_level(conn, |game| game.install_native_menu(conn, open))?
    }

    pub(in crate::game) fn install_pending_menu_open(
        &mut self,
        conn: u32,
        request: PendingMenuOpen,
    ) {
        let opened = self.in_menu_owner_level(conn, |game| match request {
            PendingMenuOpen::Native(open) => game.install_native_menu(conn, open).is_some(),
            PendingMenuOpen::ModMenu(open) => {
                matches!(game.finish_open_mod_menu(conn, open), Ok(Some(_)))
            }
            PendingMenuOpen::Block(player, entity) => {
                matches!(game.finish_player_open_menu(conn, player, entity), Ok(true))
            }
            PendingMenuOpen::Ender(player, title) => {
                matches!(game.finish_open_ender_chest(conn, player, title), Ok(true))
            }
        });
        if opened != Some(true) {
            log::warn!("deferred menu open for connection {conn} was refused");
        }
    }

    fn install_native_menu(&mut self, conn: u32, open: NativeMenuOpen) -> Option<(i32, u64)> {
        let guest = self.server.lan.as_ref()?.hub.guest(conn)?;
        if !guest.placed || !guest.participates_online() {
            return None;
        }
        if guest.disconnect_pending || (open.content.is_some() && guest.dead) {
            return None;
        }
        if self.menu_click_in_progress(conn) {
            return None;
        }
        let NativeMenuOpen {
            menu,
            title,
            items,
            replace,
            pos,
            extra,
            content,
        } = open;
        self.guest_replace_menu(conn, replace == MenuReplace::CloseFirst, |game| {
            let gi = game.guest_index(conn)?;
            // The first contents show the player's own inventory and cursor, in their turn too.
            let opened = game.with_own_record(gi, |lan, _, _| {
                if lan.hub.guests[gi].disconnect_pending {
                    return None;
                }
                let window = lan.hub.open_menu_window_at(
                    conn,
                    &menu,
                    title,
                    pos,
                    Slots::Native,
                    items,
                    extra,
                    content.as_ref().map(|c| c.kind),
                )?;
                if let Some(content) = content {
                    lan.hub.guests.update(gi, |g| {
                        let open = g.menu.as_mut().expect("the window was just installed");
                        open.blockless_data = content.data;
                        open.opening_item = content.opening_item;
                        open.opener_block = content.opener_block;
                        g.send_menu_data_snapshot();
                    });
                }
                Some((window, lan.hub.guests[gi].menu_generation))
            })?;
            game.player_menu_opening(conn);
            Some(opened)
        })
    }
    /// `ServerPlayer.containerCounter`: the window id the player's last opened menu took.
    pub(in crate::game) fn player_container_counter(
        &self,
        player: mod_api::PlayerRef,
    ) -> Option<i32> {
        if !self.common.is_logical_server() {
            return None;
        }
        Some(self.online_player_record(player)?.next_window)
    }
    /// `ServerPlayer.nextContainerCounter` with no menu after it: the counter moves as an open
    /// moves it, and nothing opens or closes.
    pub(in crate::game) fn player_next_container_counter(
        &mut self,
        player: mod_api::PlayerRef,
    ) -> Option<i32> {
        if !self.common.is_logical_server() {
            return None;
        }
        let conn = self.online_player_record(player)?.conn;
        self.server
            .lan
            .as_mut()?
            .hub
            .update_guest(conn, |guest| guest.next_container_counter())
    }
    /// `ServerPlayer.openMenu(provider, extraDataWriter)` for a content mod menu type and no
    /// block: the type's container slots show empty and their clicks reach the mods, as
    /// `open_menu`'s do; `extra` and `pos` go first in its `sunrise:menu_extra` payload, and
    /// `replace` is the provider's choice of the client's close, as for `open_menu`.
    pub fn player_open_mod_menu(
        &mut self,
        player: mod_api::PlayerRef,
        request: ModMenuOpen,
    ) -> Result<Option<MenuRef>, mod_api::OpenMenuError> {
        use mod_api::OpenMenuError;
        use sunrise_protocol::menu_extra;

        let ModMenuOpen {
            menu,
            title,
            extra,
            data,
            pos,
            opening_hand,
            replace,
            ..
        } = request;
        let title = title.try_into_engine(&())?;
        let id = mod_api::__private::menu_key_id(menu).ok_or(OpenMenuError::NotModMenu)?;
        let kind =
            sunrise_level::containers::mod_menu_type(&id).ok_or(OpenMenuError::NotModMenu)?;
        if kind.owned_slots.is_some() && kind.crafting.is_some() {
            return Err(OpenMenuError::OwnedSlotsWithCrafting);
        }
        if extra.len() > menu_extra::MAX_DATA {
            return Err(OpenMenuError::TooLarge);
        }
        if data.len() != kind.data_slots {
            return Err(OpenMenuError::DataCountMismatch);
        }
        let title = component_nbt(&title.component());
        sunrise_protocol::buf::check_network_nbt(&title)
            .map_err(|_| OpenMenuError::TitleNotEncodable)?;
        // A live handle of a connected player; a fake player has no connection.
        let guest = self
            .online_player_record(player)
            .filter(|g| !g.dead && !g.disconnect_pending)
            .ok_or(OpenMenuError::PlayerUnavailable)?;
        let revision = if !data.is_empty() || !kind.player_inventory || kind.slots == 0 {
            menu_extra::LIVE_DATA_REVISION
        } else {
            1 // Client must have negotiated menu_extra revision 1 or higher.
        };
        if !sunrise_level::containers::menu_reaches(&guest.brand, &id)
            || guest.sender.menu_extra_rev < revision
        {
            return Err(OpenMenuError::PlayerUnavailable);
        }
        let conn = guest.conn;
        self.server
            .lan
            .as_ref()
            .and_then(|host| host.hub.registry_id("minecraft:menu", &id))
            .ok_or(OpenMenuError::NotModMenu)?;
        // A speculative quick move or merchant action may still own the window.
        if self.menu_click_in_progress(conn) {
            return Err(OpenMenuError::ClickInProgress);
        }
        let open = PendingModMenuOpen {
            player,
            id: id.to_string(),
            kind,
            title,
            extra,
            data,
            pos,
            opening_hand,
            replace,
        };
        if self.menu_open_waits(conn) {
            let staged = self.defer_menu_open(conn, PendingMenuOpen::ModMenu(open));
            return Ok(Some(menu_ref(player, staged, 0)));
        }
        self.in_menu_owner_level(conn, |game| game.finish_open_mod_menu(conn, open))
            .ok_or(OpenMenuError::PlayerUnavailable)?
    }

    fn finish_open_mod_menu(
        &mut self,
        conn: u32,
        open: PendingModMenuOpen,
    ) -> Result<Option<MenuRef>, mod_api::OpenMenuError> {
        use mod_api::OpenMenuError;
        use sunrise_protocol::menu_extra;

        let PendingModMenuOpen {
            player,
            id,
            kind,
            title,
            extra,
            data,
            pos,
            opening_hand,
            replace,
        } = open;
        let guest = self
            .online_player_record(player)
            .filter(|g| g.conn == conn && !g.dead && !g.disconnect_pending)
            .ok_or(OpenMenuError::PlayerUnavailable)?;
        let revision = if !data.is_empty() || !kind.player_inventory || kind.slots == 0 {
            menu_extra::LIVE_DATA_REVISION
        } else {
            1
        };
        if !sunrise_level::containers::menu_reaches(&guest.brand, &id)
            || guest.sender.menu_extra_rev < revision
        {
            return Err(OpenMenuError::PlayerUnavailable);
        }
        self.server
            .lan
            .as_ref()
            .and_then(|host| host.hub.registry_id("minecraft:menu", &id))
            .ok_or(OpenMenuError::NotModMenu)?;
        if self.menu_click_in_progress(conn) {
            return Err(OpenMenuError::ClickInProgress);
        }
        if !self
            .guest_may_open(conn, &id, pos)
            .ok_or(OpenMenuError::PlayerUnavailable)?
        {
            return Ok(None);
        }
        let guest = self
            .online_player_record(player)
            .filter(|g| g.conn == conn && !g.dead && !g.disconnect_pending)
            .ok_or(OpenMenuError::PlayerUnavailable)?;
        let opening_item = opening_hand.map(|hand| (hand, self.held_by(guest, hand).1.cloned()));
        let pos = pos.map(|[x, y, z]| (x, y, z));
        let opener_block = pos
            .and_then(|pos| self.menu_opener_block(pos, &id))
            .map(Into::into);
        let (window, generation) = self
            .install_native_menu(
                conn,
                NativeMenuOpen {
                    menu: id,
                    title,
                    items: vec![None; kind.slots],
                    replace,
                    pos,
                    extra: Some(extra),
                    content: Some(ModMenuData {
                        kind,
                        data,
                        opening_item,
                        opener_block,
                    }),
                },
            )
            .ok_or(OpenMenuError::PlayerUnavailable)?;
        Ok(Some(menu_ref(player, window, generation)))
    }
    /// Internal handle lookup for the native menu `uuid` has open as window `window`, for
    /// server-side recipe-book and slot-backing paths.
    pub fn native_menu_ref(&self, uuid: u128, window: i32) -> Option<MenuRef> {
        let conn = self.native_conn(uuid)?;
        self.server
            .lan
            .as_ref()?
            .hub
            .guest(conn)?
            .menu
            .as_ref()
            .filter(|m| m.window == window && m.slots == Slots::Native && !m.removed)?;
        self.player_container_menu(self.player(uuid)?)
            .filter(|menu| menu.fields().window == window)
    }
    /// Replaces what the player's open native menu shows, without reopening it
    /// (`AbstractContainerMenu.sendAllDataToRemote`): the whole contents in one packet.
    pub(in crate::game) fn menu_set_items(
        &mut self,
        menu: MenuRef,
        items: Vec<Option<ItemStack>>,
    ) -> Result<(), mod_api::MenuItemsRefusal> {
        use mod_api::MenuItemsRefusal;
        if !self.common.is_logical_server() {
            return Err(MenuItemsRefusal::Client);
        }
        if self.common.native_lifecycle.epoch().is_none() {
            return Err(MenuItemsRefusal::Unavailable);
        }
        let player = menu.fields().player;
        let window = menu.fields().window;
        if self.online_player_record(player).is_none() {
            return Err(MenuItemsRefusal::NoPlayer);
        }
        let guest = self
            .native_menu_guest(menu)
            .ok_or(MenuItemsRefusal::NotOpen)?;
        let conn = guest.conn;
        let open = guest
            .menu
            .as_ref()
            .filter(|m| m.slots == Slots::Native && !m.removed)
            .ok_or(MenuItemsRefusal::NotOpen)?;
        if open.owned_slots().is_some() {
            return Err(MenuItemsRefusal::OwnedSlots);
        }
        let size = open.size;
        let gi = self.guest_index(conn).ok_or(MenuItemsRefusal::NoPlayer)?;
        let mut items = items;
        items.resize(size, None);
        let transactional = !self.common.transactions.is_empty();
        if transactional {
            self.journal_menus(player);
            self.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(gi, |g| g.native_menu = items);
        } else {
            self.with_own_record(gi, |host, _, _| {
                host.hub
                    .guests
                    .update(gi, |g| g.native_menu = items.clone());
                host.hub.send_menu_content(gi, items);
                Some(())
            });
        }
        if transactional {
            self.reply_menu_click(
                conn,
                sunrise_gameplay_common::game::lan_menu_data::ClickReply::Resend(window),
            );
        }
        Ok(())
    }
}

impl GuestMenus {
    pub(in crate::game::server_side) fn menu_holds(
        &self,
        menu: Option<&Menu>,
        pos: ContainerPos,
        generation: u64,
    ) -> bool {
        self.menu_heard
            .as_ref()
            .zip(menu)
            .is_some_and(|(opened, live)| {
                opened.window == live.window && opened.openers.contains(&Some((pos, generation)))
            })
    }
}

/// The menu `conn` has open, whichever way the game keeps it: a block's
/// container, a trade, a crafting table, a workstation, a lectern or a
/// mount's inventory. Not the player's own inventory menu.
pub fn open_menu_of(host: &LanHost, conn: u32) -> Option<(i32, &str, Option<[i32; 3]>)> {
    let g = host.hub.guest(conn)?;
    let play = host.play.get(&conn);
    let menus = host.menus.get(&conn);
    if let Some(m) = &g.menu {
        // A vehicle's menu is keyed by a stand-in, not a block.
        let pos = m
            .pos
            .filter(|p| p.1 != sunrise_sim::lan::menu::VEHICLE_KEY_Y)
            .map(|p| [p.0, p.1, p.2]);
        return Some((m.window, &m.registry_id, pos));
    }
    let (window, menu, pos) = if let Some(gm) = menus.and_then(|m| m.merchant.as_ref()) {
        (gm.window, "minecraft:merchant", None)
    } else if let Some((window, _, _, pos)) = menus.and_then(|m| m.crafting.table.as_ref()) {
        (*window, "minecraft:crafting", Some(*pos))
    } else if let Some(st) = menus.and_then(|m| m.station.as_ref()) {
        (
            st.window,
            sunrise_gameplay_common::game::lan_station_data::menu_of(st.kind)?.0,
            Some(st.pos),
        )
    } else if let Some(l) = play.and_then(|p| p.lectern.as_ref()) {
        (
            l.window,
            "minecraft:lectern",
            Some([l.key.0, l.key.1, l.key.2]),
        )
    } else {
        let m = play.and_then(|p| p.mount_window.as_ref())?;
        (m.window, MOUNT_MENU, None)
    };
    Some((window, menu, pos))
}

/// Where the game calls the menu hook points.
impl crate::game::ServerGame<'_> {
    /// `ServerPlayer.openMenu` / `openHorseInventory`: called only after
    /// constructing the menu, queuing its open packet and sending full contents.
    /// Remember it before dispatch so a hook can close it synchronously.
    pub fn player_menu_opening(&mut self, conn: u32) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let Some((window, menu, pos)) = open_menu_of(host, conn) else {
            return;
        };
        let Some(guest) = host.hub.guest(conn) else {
            return;
        };
        let openers = guest.menu.as_ref().map_or([None; 2], |m| {
            let positions = match m.slots {
                Slots::Block(at) => [Some(at), None],
                Slots::Double(first, second) => [Some(first), Some(second)],
                Slots::Ender => [m.pos, None],
                Slots::Native => [None; 2],
            };
            positions.map(|pos| {
                pos.and_then(|pos| {
                    self.common
                        .world
                        .block_entities
                        .generation(&pos)
                        .map(|generation| (pos, generation))
                })
            })
        });
        let menu = HeardMenu {
            window,
            generation: guest.menu_generation,
            menu: menu.to_owned(),
            pos,
            openers,
        };
        host.menus.entry(conn).or_default().menu_heard = Some(menu.clone());
        self.player_menu_changed(conn, menu, true);
    }

    /// `conn`'s tracked menu closes with the guest's state in place
    /// (`ServerPlayer.doCloseContainer`). Close listeners are checked independently
    /// of open listeners.
    pub(in crate::game) fn player_menu_closing(&mut self, conn: u32, window: i32, generation: u64) {
        let heard = self
            .server
            .lan
            .as_mut()
            .and_then(|host| host.menus.get_mut(&conn))
            .and_then(|p| {
                p.owned_menu = None;
                p.menu_heard
                    .take_if(|menu| menu.window == window && menu.generation == generation)
            });
        if let Some(old) = heard {
            self.player_menu_changed(conn, old, false);
        }
    }

    fn player_menu_changed(&mut self, conn: u32, menu: HeardMenu, open: bool) {
        let hook = if open {
            mod_api::HookId::PlayerContainerOpen
        } else {
            mod_api::HookId::PlayerContainerClose
        };
        let heard = mod_api::__private::listened(hook);
        let openers = menu
            .openers
            .map(|opener| opener.filter(|&(pos, _)| self.common.mod_menu_at(pos).is_some()));
        if !heard && openers == [None; 2] {
            return;
        }
        let Some(guest) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
        else {
            return;
        };
        let uuid = guest.uuid;
        let name = (heard && open).then(|| guest.name.clone());
        let dimension = self.common.dim.id();
        self.player_hook(conn, move |g| {
            for (pos, generation) in openers.into_iter().flatten() {
                g.mod_container_player_opener_for(pos, open, Some(generation));
            }
            if heard {
                let player = g.player(uuid);
                let handle = player.and_then(|player| {
                    let closing = menu_ref(player, menu.window, menu.generation);
                    g.native_menu_guest(closing)?;
                    Some(closing)
                });
                if let Some(name) = name {
                    g.native_player_container_open(
                        uuid, &name, dimension, &menu.menu, menu.pos, handle,
                    );
                } else if !open && let Some(player) = player {
                    g.native_player_container_close(player, &menu.menu, menu.pos, handle);
                }
            }
        });
    }

    /// Runs a player hook with the guest's state in place, or as it is when
    /// the guest has no body to swap in.
    fn player_hook(&mut self, conn: u32, f: impl FnOnce(&mut Self)) {
        let mut f = Some(f);
        if self.server.lan_acting != Some(conn) {
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

    /// Delivers a detached `AbstractContainerMenu.removed` event: ends backed slot places
    /// and invokes the owning `MenuPart::menu_removed` (with `player: None` if the player is
    /// already gone).
    pub(in crate::game) fn deliver_menu_removed(&mut self, event: NativeMenuEvent) {
        let NativeMenuEvent::Closed {
            uuid,
            window,
            menu,
            block,
            opening_hand,
            owned_slots,
            ..
        } = event
        else {
            return;
        };
        self.menu_places_closed(uuid, window);
        if mod_api::__private::has_part_hook(mod_api::HookId::MenuRemoved) {
            use sunrise_gameplay_common::game::api_values::TryIntoApi;
            let owned_slots: Vec<_> = owned_slots
                .iter()
                .map(|slot| slot.as_ref().and_then(|stack| stack.try_into_api().ok()))
                .collect();
            let player = self.player(uuid);
            self.native_menu_removed(mod_api::__private::MenuRemovedCall {
                menu: &menu,
                block: block.as_deref(),
                player,
                uuid,
                window,
                opening_hand,
                owned_slots: &owned_slots,
            });
        }
    }

    /// The notices the hub collected, dispatched with the hub in `self.lan` and no guest turn active.
    pub fn run_native_menu_events(&mut self) {
        loop {
            let Some(host) = self.server.lan.as_mut() else {
                return;
            };
            if host.hub.native_menu_events.is_empty() {
                return;
            }
            match host.hub.native_menu_events.remove(0) {
                NativeMenuEvent::ReturnCarried { conn, item } => {
                    if let Some(gi) = self.guest_index(conn) {
                        self.lan_place_back(gi, vec![item]);
                    }
                }
                NativeMenuEvent::ReturnProcessingInputs {
                    conn,
                    items,
                    drop_instead,
                } => {
                    if let Some(gi) = self.guest_index(conn) {
                        let dead = self
                            .server
                            .lan
                            .as_ref()
                            .is_some_and(|host| host.hub.guests[gi].dead);
                        if drop_instead || dead {
                            self.drop_guest_items(gi, items);
                        } else {
                            self.lan_place_back(gi, items);
                        }
                    }
                }
                event @ NativeMenuEvent::Closed { .. } => {
                    self.deliver_menu_removed(event);
                }
                // A write to a backed menu's storage (#3385): its slots looked at again.
                NativeMenuEvent::PlacesChanged { uuid, window } => {
                    self.menu_places_changed(uuid, window)
                }
            }
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use sunrise_gameplay_common::game::server_side::native_item_places::tests::guest;

    use sunrise_gameplay_common::game::server_side::native_menus::tests::MOD_MENU;

    use super::*;
    use crate::game::server_side::native_item_places::tests::open_to_lan;
    use mod_api::ServerHost;
    use std::sync::{Arc, mpsc};
    use sunrise_sim::lan::{Out, PlayerSender};

    /// Guest 1 (UUID 123) on a world open to LAN, and what their client is sent.
    pub fn watched_guest(game: &mut crate::game::ServerGame<'_>) -> mpsc::Receiver<Out> {
        open_to_lan(game);
        watch_guest_in_hub(game)
    }

    pub fn watch_guest_in_hub(game: &mut crate::game::ServerGame<'_>) -> mpsc::Receiver<Out> {
        let (tx, rx) = mpsc::channel();
        let mut player = guest(1);
        player.sender = PlayerSender::new(tx, Arc::default());
        game.server.lan.as_mut().unwrap().hub.guests.push(player);
        rx
    }

    pub fn open(
        game: &mut crate::game::ServerGame<'_>,
        rows: u8,
        title: &str,
        replace: MenuReplace,
    ) -> MenuRef {
        let player = game.player(123).unwrap();
        let host: &mut dyn ServerHost = game;
        player
            .open_chest_menu(
                host,
                rows,
                mod_api::Text::literal(title),
                Vec::new(),
                replace,
            )
            .unwrap()
    }

    /// [`watched_guest`] on a Sunrise client that reads a menu opened with no block, and the
    /// key of [`MOD_MENU`].
    pub fn mod_menu_guest(
        game: &mut crate::game::ServerGame<'_>,
    ) -> (mpsc::Receiver<Out>, mod_api::MenuKey) {
        let rx = watched_guest(game);
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.brand = "sunrise".into();
            g.sender.menu_extra_rev = 1;
        });
        let key = super::super::native_guest_turn_tests::register_test_menu(game, MOD_MENU);
        rx.try_iter().for_each(drop); // Registry setup sent reload packets, not part of these opens.
        (rx, key)
    }

    #[cfg(test)]
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

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_menu_refs_common::NativeMenuRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
