use crate::game::ServerGame;
use sunrise_sim::lan::guest::Guest;

impl ServerGame<'_> {
    /// The leave lines of a guest the hub removed and broadcast the leave line of, and whether
    /// the mods hear that they logged out.
    pub(in crate::game) fn announce_guest_left(&mut self, guest: &Guest) -> bool {
        // A fake silently displaced by a real login was never announced as online.
        if guest.is_fake() && !guest.disconnect_pending {
            return false;
        }
        // A held player was recorded as joined at its login: it leaves too.
        if self.common.is_dedicated_server() && (guest.joined() || guest.held) {
            crate::game::dedicated::events::record(
                "leave",
                &guest.name,
                guest.uuid,
                serde_json::json!({ "reason": guest.left_reason }),
            );
            if guest.held && !guest.joined() {
                // Never announced, so no "left the game" either.
                crate::game::dedicated::log(
                    "INFO",
                    &sunrise_sim::lan::arrivals::dropped_line(&guest.name, &guest.left_reason),
                );
            } else {
                crate::game::dedicated::log(
                    "INFO",
                    &format!("{} lost connection: {}", guest.name, guest.left_reason),
                );
            }
            // A player moved to another server without `announce` was not
            // announced as leaving, so the line is not logged either; a held
            // player was never announced at all.
            if guest.joined() && !guest.quiet_leave {
                crate::game::dedicated::log(
                    "INFO",
                    &format!("System chat: {} left the game", guest.name),
                );
            }
        }
        guest.joined()
            && self
                .server
                .lan
                .as_ref()
                .is_some_and(|host| !host.players_told_left)
    }
    /// Fire `player_logged_out` for a departing guest outside an active level turn (for example
    /// one quitting from the End credits or when their level failed to open), keeping them listed
    /// in join order (`listed_behind`) and reachable as online (`Hub::departing`) so listener
    /// edits persist if this server saves the player.
    pub(in crate::game) fn native_departing_guest_logout(
        &mut self,
        guest: Box<Guest>,
    ) -> Box<Guest> {
        let (uuid, name) = (guest.uuid, guest.name.clone());
        self.with_relisted_departing_guest(guest, |game| game.native_player_logged_out(uuid, &name))
    }

    /// `PlayerContainerEvent.Close` for the menu a departing guest held when their level could
    /// not be entered: told once, with no live menu handle (the level and its storage are
    /// unavailable), before the tracked menu is forgotten.
    pub(in crate::game) fn native_departing_guest_menu_closed(
        &mut self,
        guest: Box<Guest>,
        window: i32,
        generation: u64,
    ) -> Box<Guest> {
        let conn = guest.conn;
        self.with_relisted_departing_guest(guest, |game| {
            game.player_menu_closing(conn, window, generation)
        })
    }

    /// Run `tell` with the departing guest listed in join order and reachable as online
    /// (`Hub::departing`), then take them out again.
    fn with_relisted_departing_guest(
        &mut self,
        mut guest: Box<Guest>,
        tell: impl FnOnce(&mut Self),
    ) -> Box<Guest> {
        let conn = guest.conn;
        let Some(host) = self.server.lan.as_mut() else {
            return guest;
        };
        guest.disconnect_pending = true;
        // Right behind the last of the players listed ahead of them who are still here.
        let at = host
            .hub
            .guests
            .iter()
            .rposition(|g| guest.listed_behind.contains(&g.conn))
            .map_or(0, |i| i + 1);
        host.hub.guests.insert(at, *guest);
        host.hub.departing = Some(conn);
        tell(self);
        let Some(host) = self.server.lan.as_mut() else {
            unreachable!("LanHost dropped inside a tick; closes are deferred")
        };
        host.hub.departing = None;
        let at = host
            .hub
            .guests
            .connection_index(conn)
            .expect("a leaving player stays listed while the mods hear it");
        Box::new(host.hub.guests.remove(at))
    }
}
