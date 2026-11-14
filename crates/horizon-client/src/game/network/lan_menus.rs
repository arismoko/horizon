//! The menus a LAN guest has open that the game runs for them, kept outside the guest's
//! `GuestPlay`. A guest's turn (`as_guest`) lifts the whole `GuestPlay` out of the host while it
//! runs, so a menu owner kept there is out of reach of anything that runs inside the turn: another
//! nested turn, a hook, an action that needs to find the menu to admit or undo. These owners stay
//! in the host, by connection, for as long as the guest is here.
use super::{
    lan::LanHost, lan_crafting::GuestCrafting, lan_stations::GuestStation,
    transactions::decision::MenuOwner,
};

/// One guest's menu owners: the inventory menu, the crafting table and the furnace or brewing
/// stand (`GuestCrafting`), and the workstation. Each owner counts every access, so an action can
/// tell that its menu was touched.
#[derive(Default)]
pub(in crate::game) struct GuestMenus {
    /// The inventory menu, the crafting table and the machine menu.
    pub(in crate::game) crafting: GuestCrafting,
    /// The workstation menu the guest has open (game/network/lan_stations.rs).
    pub(in crate::game) station: MenuOwner<Option<GuestStation>>,
    /// Names the hub's container clicks of this guest, which have no menu value of their own: a
    /// click while another of the guest's is being decided is refused as busy.
    pub(in crate::game) hub: MenuOwner<()>,
}

impl LanHost {
    /// Forget `conn`'s menu owners: the guest left or was removed, so nothing can name them again.
    pub(in crate::game) fn forget_menus(&mut self, conn: u32) -> Option<GuestMenus> {
        self.menus.remove(&conn)
    }
}
