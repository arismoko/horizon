//! Whether one player's client is shown another, by the rules the entity tracker follows, and a
//! block-change acknowledgement and a block shown to one player.
use super::{Game, native_server::PlayerRead};
use horizon_protocol::packets::play::{self, Clientbound as C};
use horizon_world::registry::BlockStateId;
use horizon_client::lan::{entity::player_shown, guest::Guest};
use mod_api::PlayerRef;

impl Game<'_> {
    /// The server's record of a player in its list, by handle.
    fn listed_guest(&self, player: PlayerRef) -> Option<&Guest> {
        (self.native_lifecycle.epoch()? == player.entity().epoch()).then_some(())?;
        let guests = &self.lan.as_ref()?.hub.guests;
        let guest = &guests[guests.identity_index(player.uuid(), player.entity().id())?];
        (guest.placed && !guest.is_fake()).then_some(guest)
    }
    /// The connection of a player a client stands behind, in their turn or out of it.
    fn player_connection(&self, player: PlayerRef) -> Option<u32> {
        if !self.is_logical_server() {
            return None;
        }
        let conn = match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => self.lan_acting?,
            PlayerRead::Guest(g) if !g.is_fake() => g.conn,
            PlayerRead::Guest(_) => return None,
        };
        let guest = self.lan.as_ref()?.hub.guest(conn)?;
        (!guest.is_fake()).then_some(conn)
    }
    /// `ackBlockChangesUpTo` for a connected player: the hub's pending ack, sent after the
    /// next tick's block changes.
    pub(super) fn player_ack_block_changes(&mut self, player: PlayerRef, sequence: i32) -> bool {
        if sequence < 0 {
            return false;
        }
        let (Some(conn), Some(host)) = (self.player_connection(player), self.lan.as_ref()) else {
            return false;
        };
        host.hub.ack(conn, sequence);
        true
    }
    /// `Player.sendBlockChange`: one `BlockUpdate` to that player's client, now.
    pub(super) fn player_show_block(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
        state: BlockStateId,
    ) -> bool {
        // BlockPos.asLong packs x and z in 26 signed bits and y in 12: a value outside wraps.
        const HORIZONTAL: std::ops::Range<i32> = -(1 << 25)..(1 << 25);
        const VERTICAL: std::ops::Range<i32> = -(1 << 11)..(1 << 11);
        let [x, y, z] = pos;
        if !(HORIZONTAL.contains(&x) && VERTICAL.contains(&y) && HORIZONTAL.contains(&z)) {
            return false;
        }
        let (Some(conn), Some(host)) = (self.player_connection(player), self.lan.as_ref()) else {
            return false;
        };
        let Some(guest) = host.hub.guest(conn) else {
            return false;
        };
        guest.send(&C::BlockUpdate(play::BlockUpdate {
            pos,
            state: state.0 as i32,
        }));
        true
    }
    /// `ServerPlayer.broadcastToPlayer` of `target` for `viewer` (`lan::entity::player_shown`).
    pub(super) fn player_sees(&self, viewer: PlayerRef, target: PlayerRef) -> Option<bool> {
        if !self.is_logical_server() {
            return None;
        }
        let (v, t) = (self.listed_guest(viewer)?, self.listed_guest(target)?);
        if v.uuid == t.uuid {
            return Some(true);
        }
        let hidden = self.lan.as_ref()?.hub.hidden.get(&t.uuid).copied();
        Some(player_shown(
            v.spectator(),
            v.permission,
            t.spectator(),
            t.watching,
            hidden,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;
    use horizon_protocol::packets::play::Clientbound as C;

    /// The highest sequence of the tick waits as the player's ack, in their turn or out of it.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_mod_acknowledges_block_changes() {
        with_guests(|game| {
            let b = game.player(13).unwrap();
            let kept = |game: &super::super::Game<'_>| {
                game.lan.as_ref().unwrap().hub.guest(13).unwrap().ack_up_to.get()
            };
            assert!(!game.player_ack_block_changes(b, -1));
            assert_eq!(kept(game), -1);
            assert!(game.player_ack_block_changes(b, 7));
            assert!(game.player_ack_block_changes(b, 3));
            assert_eq!(kept(game), 7);
            game.as_guest(13, |game| assert!(game.player_ack_block_changes(b, 9)))
                .unwrap();
            assert_eq!(kept(game), 9);
        });
    }

    /// A block shown to one player goes to their client alone and changes nothing in the world.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_block_is_shown_to_one_player() {
        with_guests(|game| {
            let b = game.player(13).unwrap();
            let (tx, rx) = std::sync::mpsc::channel();
            game.lan.as_mut().unwrap().hub.guests.update_connection(13, |g| {
                g.sender = horizon_client::lan::PlayerSender::new(tx, Default::default());
            });
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let before = game.world.block(1, 70, 1);
            assert!(game.player_show_block(b, [1, 70, 1], stone));
            assert!(!game.player_show_block(b, [1, 4000, 1], stone));
            assert!(!game.player_show_block(b, [1 << 26, 70, 1], stone));
            assert_eq!(game.world.block(1, 70, 1), before);
            let mut shown = Vec::new();
            while let Ok(horizon_client::lan::Out::Packet(body)) = rx.try_recv() {
                let mut r = horizon_protocol::buf::Reader::new(&body);
                let id = r.varint().unwrap();
                if let Ok(C::BlockUpdate(u)) = C::decode(id, r.rest()) {
                    shown.push((u.pos, u.state));
                }
            }
            assert_eq!(shown, [([1, 70, 1], stone.0 as i32)]);
        });
    }

    /// A player is seen unless a mod hid them below the viewer's level, or they spectate and
    /// the viewer doesn't.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn player_sees_follows_the_trackers_rules() {
        with_guests(|game| {
            let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
            assert_eq!(game.player_sees(a, b), Some(true));
            assert_eq!(game.player_sees(a, a), Some(true));
            assert!(game.player_hide(b, Some(3)));
            assert_eq!(game.player_sees(a, b), Some(false));
            assert_eq!(game.player_sees(b, a), Some(true));
            let guests = &mut game.lan.as_mut().unwrap().hub.guests;
            guests.update_connection(12, |g| g.permission = 3);
            assert_eq!(game.player_sees(a, b), Some(true));
            assert!(game.player_hide(b, None));
            let guests = &mut game.lan.as_mut().unwrap().hub.guests;
            guests.update_connection(13, |g| g.game_mode = 3);
            assert_eq!(game.player_sees(a, b), Some(false), "a spectator");
            assert_eq!(game.player_sees(b, a), Some(true));
            let stale = mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(
                    mod_api::WorldEpoch::new(b.entity().epoch().get() + 1),
                    b.entity().id(),
                ),
                b.uuid(),
            );
            assert_eq!(game.player_sees(a, stale), None);
        });
    }
}
