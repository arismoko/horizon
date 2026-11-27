//! The players' viewed areas (#3716, `lan::viewed_levels`) in the level in place: the squares
//! `Streaming.extra` keeps loaded for them (`lan_update_streaming`) and their own tickets
//! (`Sources::viewed`). Neither goes through the forced chunks or the mods' chunk tickets. With no
//! area on the server both are empty, without allocating.
use crate::game::ServerGame;
use sunrise_sim::ticket_levels::ViewedTicket;

impl ServerGame<'_> {
    /// The squares the areas in the level in place keep loaded and generated.
    pub(in crate::game::server_side) fn viewed_extra(&self) -> Vec<((i32, i32), i32)> {
        self.server
            .lan
            .as_ref()
            .map_or_else(Vec::new, |h| h.hub.viewed_extra(self.common.dim.id()))
    }

    /// The areas' tickets in the level in place.
    pub(in crate::game::server_side) fn viewed_tickets(&self) -> Vec<ViewedTicket> {
        self.server
            .lan
            .as_ref()
            .map_or_else(Vec::new, |h| h.hub.viewed_tickets(self.common.dim.id()))
    }
}
