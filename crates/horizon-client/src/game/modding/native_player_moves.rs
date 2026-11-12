//! `player_move` (Paper's `PlayerMoveEvent`) for the server's players: once a
//! tick, after the hub accepted their movement.
use super::Game;

/// Paper's `ServerGamePacketListenerImpl.handleMovePlayer` thresholds: a
/// move of more than 1/256 squared (1/16 of a block), a turn of more than 10
/// degrees.
const MOVED_SQUARED: f64 = 1.0 / 256.0;
const TURNED_DEGREES: f32 = 10.0;

impl Game<'_> {
    /// `player_move` for each player placed in the level in place who moved
    /// or turned enough since they were last heard; a cancelled move
    /// teleports them back to where they were heard. A player's first tick
    /// here, a teleport and a ride only move where they are heard from.
    pub(in crate::game) fn tick_player_moves(&mut self) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let listened = mod_api::__private::listened(mod_api::HookId::PlayerMove);
        let mut moved = Vec::new();
        for i in 0..host.hub.guests.len() {
            let g = &host.hub.guests[i];
            if !host.hub.here(g) || g.is_fake() {
                continue;
            }
            let now = ([g.position[0], g.position[1], g.position[2]], [g.yaw, g.pitch]);
            let rebase = g.vehicle.is_some()
                || g.awaiting_teleport.is_some()
                || g.move_heard.as_ref().is_none_or(|(dim, _, _, teleports)| {
                    *dim != g.dimension || *teleports != g.teleport_counter
                });
            if rebase || !listened {
                let heard = (g.dimension.clone(), now.0, now.1, g.teleport_counter);
                host.hub.guests.update(i, |g| g.move_heard = Some(heard));
                continue;
            }
            let Some((_, from, from_rotation, _)) = g.move_heard.clone() else {
                continue;
            };
            let d = [0, 1, 2].map(|k| now.0[k] - from[k]);
            let distance = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            let turn = (now.1[0] - from_rotation[0]).abs() + (now.1[1] - from_rotation[1]).abs();
            if distance > MOVED_SQUARED || turn > TURNED_DEGREES {
                moved.push((
                    g.conn,
                    g.uuid,
                    from,
                    from_rotation,
                    now,
                    g.teleport_counter,
                ));
            }
        }
        for (conn, uuid, from, from_rotation, (to, to_rotation), teleports) in moved {
            let mut cancel = false;
            let mut hook = |game: &mut Self| {
                game.native_player_move(
                    uuid,
                    from,
                    from_rotation,
                    to,
                    to_rotation,
                    &mut cancel,
                );
            };
            if self.lan_acting == Some(conn) {
                hook(self);
            } else if self.as_guest(conn, hook).is_none() {
                continue;
            }
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            let Some(i) = host.hub.guests.iter().position(|g| g.conn == conn) else {
                continue;
            };
            // A mod that teleported the player in the hook: the next tick hears from there.
            if host.hub.guests[i].teleport_counter != teleports {
                continue;
            }
            if cancel {
                host.hub.guests.update(i, |g| {
                    [g.yaw, g.pitch] = from_rotation;
                    g.head_yaw = g.yaw;
                });
                host.hub.teleport(i, Some(from));
            }
            let g = &host.hub.guests[i];
            let at = if cancel { from } else { to };
            let rotation = if cancel { from_rotation } else { to_rotation };
            let heard = (g.dimension.clone(), at, rotation, g.teleport_counter);
            host.hub.guests.update(i, |g| g.move_heard = Some(heard));
        }
    }
}
