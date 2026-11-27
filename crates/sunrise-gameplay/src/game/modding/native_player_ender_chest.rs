//! A server player's ender chest (`Player.getEnderChestInventory`) by slot, for mods that copy
//! or edit it whole (player data sync, `/endersee`), and its menu opened with no block.
use super::commands::items::Slot;
use super::native_menus::PendingMenuOpen;
use mod_api::{InventoryView, OpenMenuError, PlayerDataRefusal, PlayerRef};
use sunrise_core::nbt::Tag;
use sunrise_game_data::text::Text;
use sunrise_gameplay_common::game::api_values::{project_stack, stack_read_metadata};
use sunrise_gameplay_common::game::native_server::PlayerRead;
use sunrise_items::items::ItemStack;
use sunrise_sim::lan::{data::component_nbt, menu::Slots};

/// `PlayerEnderChestContainer`'s size.
const SLOTS: usize = 27;

impl crate::game::ServerGame<'_> {
    /// Lend `PlayerEnderChestContainer.getItem` over the live or saved slots.
    pub(in crate::game) fn player_ender_chest(
        &self,
        player: PlayerRef,
        read: &mut dyn for<'a> FnMut(InventoryView<'a>),
    ) -> Option<()> {
        if !self.common.is_logical_server() {
            return None;
        }
        let items = match self.player_handle_read(player)? {
            // An open ender chest menu holds the live copy until it closes.
            PlayerRead::Acting { .. } => match self
                .server
                .ender_open
                .and_then(|key| self.common.world.block_entities.get(&key))
            {
                Some(sunrise_level::containers::BlockEntity::Chest(items)) => items,
                _ => &self.server.ender_items,
            },
            PlayerRead::Guest(g) => &g.ender_items,
        };
        let stack_at = |slot| items.get(slot as usize)?.as_ref();
        read(InventoryView::new(
            0..SLOTS as u32,
            &|slot| stack_read_metadata(stack_at(slot)?).ok(),
            &|slot, metadata| project_stack(stack_at(slot)?, metadata).ok(),
        ));
        Some(())
    }

    /// `PlayerEnderChestContainer.setItem`, in the player's turn.
    pub(in crate::game) fn player_set_ender_chest_slot(
        &mut self,
        player: PlayerRef,
        slot: u32,
        stack: Option<ItemStack>,
    ) -> Result<(), PlayerDataRefusal> {
        let turn = self.native_resident_player_turn(player)?;
        if slot as usize >= SLOTS {
            return Err(PlayerDataRefusal::SlotOutOfRange {
                slot,
                last: SLOTS as u32 - 1,
            });
        }
        let set = turn.run(self, |game| {
            game.journal_player(player.uuid());
            if let Some(key) = game.server.ender_open {
                game.journal_block_entity(key);
            }
            if game.server.ender_items.len() < SLOTS {
                game.server.ender_items.resize(SLOTS, None);
            }
            game.slot_set(&Slot::Ender(slot as usize), stack.filter(|s| s.count > 0))
        });
        if set == Some(true) {
            Ok(())
        } else {
            Err(PlayerDataRefusal::Stale)
        }
    }

    /// `ServerPlayer.openMenu` of `ChestMenu.threeRows` over the player's
    /// `PlayerEnderChestContainer`, with no active chest. Inside the player's own turn their
    /// ender chest is in `Game` and their record holds the outer player's, so the menu waits
    /// for the turn's end (`open_turn_ender_menus`), as a block's menu the turn opened does.
    pub(in crate::game) fn player_open_ender_chest(
        &mut self,
        player: PlayerRef,
        title: Text,
    ) -> Result<bool, OpenMenuError> {
        if !self.common.is_logical_server() {
            return Err(OpenMenuError::PlayerUnavailable);
        }
        let conn = self
            .online_player_record(player)
            .filter(|g| g.participates_online() && !g.dead && !g.disconnect_pending)
            .ok_or(OpenMenuError::PlayerUnavailable)?
            .conn;
        if self.menu_click_in_progress(conn) {
            return Ok(false);
        }
        if self.menu_open_waits(conn) {
            self.defer_menu_open(conn, PendingMenuOpen::Ender(player, title));
            return Ok(true);
        }
        Ok(self.request_ender_menu(conn, component_nbt(&title.component())))
    }

    pub(in crate::game::server_side) fn finish_open_ender_chest(
        &mut self,
        conn: u32,
        player: PlayerRef,
        title: Text,
    ) -> Result<bool, OpenMenuError> {
        self.online_player_record(player)
            .filter(|g| {
                g.conn == conn && g.participates_online() && !g.dead && !g.disconnect_pending
            })
            .ok_or(OpenMenuError::PlayerUnavailable)?;
        Ok(self.request_ender_menu(conn, component_nbt(&title.component())))
    }

    fn request_ender_menu(&mut self, conn: u32, title: Tag) -> bool {
        if self.server.lan_guest_swaps.contains(&conn) {
            self.server.lan_ender_menus.push((conn, title));
            return true;
        }
        self.open_ender_menu(conn, title)
    }

    /// The ender chest menus `player_open_ender_chest` asked for in guest `conn`'s turn,
    /// opened now that the turn is over, in the order asked (the last one stays open).
    pub(in crate::game::server_side) fn open_turn_ender_menus(&mut self, conn: u32) {
        if !self.server.lan_ender_menus.iter().any(|(c, _)| *c == conn) {
            return;
        }
        let (mine, others) = std::mem::take(&mut self.server.lan_ender_menus)
            .into_iter()
            .partition(|(c, _)| *c == conn);
        self.server.lan_ender_menus = others;
        for (_, title) in mine {
            self.open_ender_menu(conn, title);
        }
    }

    /// Opens the ender chest menu for guest `conn` out of every turn: the mods may cancel it,
    /// the menu they had open closes as `ServerPlayer.openMenu` closes it, and the slots are
    /// the guest's own 27 (`Slots::Ender`, no block), which the game's container clicks move.
    fn open_ender_menu(&mut self, conn: u32, title: Tag) -> bool {
        self.in_menu_owner_level(conn, |game| game.install_ender_menu(conn, title))
            .unwrap_or(false)
    }

    fn install_ender_menu(&mut self, conn: u32, title: Tag) -> bool {
        const MENU: &str = "minecraft:generic_9x3";
        if self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .is_none_or(|g| g.dead)
            || self.guest_may_open(conn, MENU, None) != Some(true)
        {
            return false;
        }
        self.guest_replace_menu(conn, true, |game| {
            let Some(host) = game.server.lan.as_mut() else {
                return false;
            };
            let Some(mut items) = host.hub.guest(conn).map(|g| g.ender_items.clone()) else {
                return false;
            };
            items.resize(SLOTS, None);
            let opened = host
                .hub
                .open_menu_window(conn, MENU, title, None, Slots::Ender, items, None)
                .is_some();
            if opened {
                game.player_menu_opening(conn);
            }
            opened
        })
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
