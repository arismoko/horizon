//! The menus a LAN guest has open that the game runs for them, kept outside the guest's
//! `GuestPlay`. A guest's turn (`as_guest`) lifts the whole `GuestPlay` out of the host while it
//! runs, so a menu owner kept there is out of reach of anything that runs inside the turn: another
//! nested turn, a hook, an action that needs to find the menu to admit or undo. These owners stay
//! in the host, by connection, for as long as the guest is here.
use std::collections::VecDeque;

use super::{lan::LanHost, lan_crafting::GuestCrafting, native_menus::PendingMenuOpen};

use sunrise_gameplay_common::game::{
    lan_menu_data::{ClickReply, GuestMerchant, GuestStation, MenuClickReply},
    native_menu_data::HeardMenu,
    transactions::decision::MenuOwner,
};

/// One guest's menu owners: the inventory menu, the crafting table and the furnace or brewing
/// stand (`GuestCrafting`), and the workstation. Each owner counts every access, so an action can
/// tell that its menu was touched.
#[derive(Default)]
pub struct GuestMenus {
    /// The inventory menu, the crafting table and the machine menu.
    pub crafting: GuestCrafting,
    /// The workstation menu the guest has open (game/network/lan_stations.rs).
    pub station: MenuOwner<Option<GuestStation>>,
    /// The transient menu engine, kept between clicks for drag state.
    pub(in crate::game) owned_menu: Option<(u64, Box<sunrise_sim::menu::Menu>)>,
    /// The trading window is live even while the guest's player state is on loan.
    pub merchant: MenuOwner<Option<GuestMerchant>>,
    /// Names the hub's container clicks of this guest, which have no menu value of their own: a
    /// click while another of the guest's is being decided is refused as busy.
    pub(in crate::game) hub: MenuOwner<()>,
    /// The successful open already announced to mods, retained through a loan of `GuestPlay`.
    pub(in crate::game) menu_heard: Option<HeardMenu>,
    /// Installed from the start of `removed` until the general Close returns.
    pub(in crate::game) closing: bool,
    /// The storage a mod backed its open menu's slots with (`MenuRef::back_slots`);
    /// None for every menu no mod backed.
    pub(in crate::game) slot_places:
        Option<sunrise_gameplay_common::game::native_menu_values::MenuSlotPlaces>,
}

struct QueuedMenuOpen {
    identity: i32,
    open: Option<Box<PendingMenuOpen>>,
}

/// Per-connection FIFO from root commit through publication. A root reserves the positions
/// of its opens before any callbacks reenter. The wire counter belongs to installation alone.
#[derive(Default)]
pub(in crate::game) struct MenuPublications {
    queue: VecDeque<QueuedMenuOpen>,
    holds: usize,
    publishing: bool,
}

/// Moving or replacing the player's current menu holds publication until teardown and handoff
/// complete. The hold identifies the same connection on release.
pub(in crate::game) struct MenuOpenHold(u32);

impl crate::game::ServerGame<'_> {
    /// The addressed player's menu owns its level. Run its lifecycle there, even when
    /// a callback from another level requested it, and restore the caller's level.
    pub(in crate::game) fn in_menu_owner_level<R>(
        &mut self,
        conn: u32,
        run: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        let guest = self.server.lan.as_ref()?.hub.guest(conn)?;
        let dim = super::Dim::from_id(&guest.dimension);
        if dim == self.common.dim {
            return Some(run(self));
        }
        let back = self.enter_command_level(dim)?;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(self)));
        self.enter_command_level(back);
        match result {
            Ok(value) => Some(value),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    pub(in crate::game::server_side) fn reply_predicted_menu_click(
        &mut self,
        conn: u32,
        packet: &sunrise_protocol::packets::play::ContainerClick,
        changed: Vec<usize>,
    ) {
        if self.common.transactions.is_empty() {
            if let Some(host) = self.server.lan.as_mut() {
                host.hub
                    .inventory_clicked(conn, packet, &self.common.world, &changed);
            }
        } else {
            self.reply_menu_click(conn, ClickReply::Predicted(packet.clone(), changed));
        }
    }

    pub(in crate::game) fn reply_menu_click_for(
        &mut self,
        conn: u32,
        generation: u64,
        reply: ClickReply,
    ) {
        self.after_commit(
            sunrise_gameplay_common::game::transactions::After::MenuClick(Box::new(
                MenuClickReply {
                    conn,
                    generation,
                    reply,
                },
            )),
        );
    }

    pub(in crate::game) fn reply_menu_click(&mut self, conn: u32, reply: ClickReply) {
        if let Some(guest) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
        {
            self.reply_menu_click_for(conn, guest.menu_generation, reply);
        }
    }

    /// Run `removed` only after the outermost commit. An abort keeps the current menu.
    pub(in crate::game) fn request_menu_close(&mut self, conn: u32, window: i32, generation: u64) {
        self.reply_menu_click_for(conn, generation, ClickReply::Close(window));
    }

    /// An open waits for its transaction or an older publication on the same connection.
    pub(in crate::game) fn menu_open_waits(&self, conn: u32) -> bool {
        !self.common.transactions.is_empty() || self.server.menu_publications.contains_key(&conn)
    }

    pub(in crate::game) fn defer_menu_open(&mut self, conn: u32, open: PendingMenuOpen) -> i32 {
        self.server.staged_menu_open = match self.server.staged_menu_open.checked_sub(1) {
            Some(next) if next < 0 => next,
            _ => -1,
        };
        let identity = self.server.staged_menu_open;
        self.after_commit(
            sunrise_gameplay_common::game::transactions::After::MenuOpen(
                conn,
                identity,
                Box::new(open),
            ),
        );
        identity
    }

    pub(in crate::game) fn reserve_menu_publication(&mut self, conn: u32, identity: i32) {
        self.server
            .menu_publications
            .entry(conn)
            .or_default()
            .queue
            .push_back(QueuedMenuOpen {
                identity,
                open: None,
            });
    }

    pub(in crate::game) fn publish_menu_open(
        &mut self,
        conn: u32,
        identity: i32,
        open: Box<PendingMenuOpen>,
    ) {
        let queue = &mut self.server.menu_publications.entry(conn).or_default().queue;
        if let Some(entry) = queue.iter_mut().find(|entry| entry.identity == identity) {
            entry.open = Some(open);
        } else {
            queue.push_back(QueuedMenuOpen {
                identity,
                open: Some(open),
            });
        }
        self.drain_menu_opens(conn);
    }

    pub(in crate::game) fn hold_menu_opens(&mut self, conn: u32) -> MenuOpenHold {
        self.server.menu_publications.entry(conn).or_default().holds += 1;
        MenuOpenHold(conn)
    }

    /// Installation is complete. Keep a hold through notifications only if there are
    /// older opens to publish afterwards. An empty hold must not defer a fresh open
    /// from a block's `startOpen` event while its installed menu is still current.
    pub(in crate::game) fn release_empty_menu_open_hold(
        &mut self,
        hold: MenuOpenHold,
    ) -> Option<MenuOpenHold> {
        if !self.server.menu_publications[&hold.0].queue.is_empty() {
            return Some(hold);
        }
        self.resume_menu_opens(hold);
        None
    }

    /// The player is leaving: the opens held back never publish.
    pub(in crate::game) fn discard_menu_opens(&mut self, MenuOpenHold(conn): MenuOpenHold) {
        if let Some(stream) = self.server.menu_publications.get_mut(&conn) {
            stream.queue.clear();
            stream.holds -= 1;
        }
        self.drain_menu_opens(conn);
    }

    pub(in crate::game) fn resume_menu_opens(&mut self, MenuOpenHold(conn): MenuOpenHold) {
        self.server.menu_publications.get_mut(&conn).unwrap().holds -= 1;
        self.drain_menu_opens(conn);
    }

    pub(in crate::game) fn drain_menu_opens(&mut self, conn: u32) {
        let Some(stream) = self.server.menu_publications.get_mut(&conn) else {
            return;
        };
        if stream.holds != 0 || stream.publishing {
            return;
        }
        stream.publishing = true;
        loop {
            let stream = self.server.menu_publications.get_mut(&conn).unwrap();
            if stream.holds != 0 {
                break;
            }
            let Some(open) = stream.queue.front().and_then(|entry| entry.open.as_deref()) else {
                break;
            };
            // Ender slots are in Game during the owner's turn. Waiting for the record
            // to come back must not let another open overtake this one.
            if matches!(open, PendingMenuOpen::Ender(..))
                && self.server.lan_guest_swaps.contains(&conn)
            {
                break;
            }
            let open = stream
                .queue
                .pop_front()
                .and_then(|entry| entry.open)
                .unwrap();
            self.install_pending_menu_open(conn, *open);
        }
        let stream = self.server.menu_publications.get_mut(&conn).unwrap();
        stream.publishing = false;
        if stream.holds == 0 && stream.queue.is_empty() {
            self.server.menu_publications.remove(&conn);
        }
    }

    pub(in crate::game) fn publish_menu_click(&mut self, reply: MenuClickReply) {
        use sunrise_protocol::packets::play::{Clientbound as C, SetPlayerInventory};
        let MenuClickReply {
            conn,
            generation,
            reply,
        } = reply;
        if let ClickReply::Close(window) | ClickReply::ClientClose(window) = reply {
            self.close_guest_window(
                conn,
                window,
                generation,
                matches!(reply, ClickReply::Close(_)),
            );
            return;
        }
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let Some(i) = host.hub.guests.connection_index(conn) else {
            return;
        };
        let guest = &host.hub.guests[i];
        if guest.menu_generation != generation || guest.disconnect_pending {
            return;
        }
        match reply {
            ClickReply::Predicted(packet, changed) => {
                let window = host.hub.guests[i].menu.as_ref().map_or(0, |m| m.window);
                if packet.container == window {
                    host.hub
                        .inventory_clicked(conn, &packet, &self.common.world, &changed);
                }
            }
            ClickReply::Resend(window) => {
                self.with_own_record(i, |host, _, world| {
                    host.hub.resend_menu(conn, window, world);
                    Some(())
                });
            }
            ClickReply::Inventory => host.hub.send_inventory(i),
            ClickReply::Offhand => {
                let offhand = guest.offhand.clone();
                let item = host.hub.stack(&offhand);
                host.hub.guests.update(i, |g| {
                    g.send(&C::SetPlayerInventory(SetPlayerInventory {
                        slot: 40,
                        item,
                    }));
                    g.menu_sent[45] = offhand;
                });
            }
            ClickReply::Close(_) | ClickReply::ClientClose(_) => unreachable!(),
        }
    }
}

impl LanHost {
    /// Forget `conn`'s menu owners: the guest left or was removed, so nothing can name them again.
    pub(in crate::game) fn forget_menus(&mut self, conn: u32) -> Option<GuestMenus> {
        self.menus.remove(&conn)
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
