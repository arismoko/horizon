//! A guest's flying kicks, in their own module so `hub.rs` keeps its size. The
//! `handlePlayerPositionChange` collision check is in `move_collision`.
use super::*;
impl Hub {
    /// `ServerGamePacketListenerImpl.tickPlayer`'s flying check: a player the
    /// last move left hovering for more than `getMaximumFlyingTicks` ticks is
    /// kicked.
    pub(super) fn tick_floating(&mut self, i: usize, events: &mut Vec<HostEvent>) {
        let kick = self.guests.update(i, |g| {
            if g.client_is_floating && !g.sleeping && g.vehicle.is_none() && !g.dead {
                g.above_ground_ticks += 1;
                if g.above_ground_ticks > maximum_flying_ticks(PLAYER_GRAVITY) {
                    events.push(HostEvent::Warn(format!(
                        "{} was kicked for floating too long!",
                        g.name
                    )));
                    return Some(g.conn);
                }
            } else {
                g.client_is_floating = false;
                g.above_ground_ticks = 0;
            }
            None
        });
        if let Some(conn) = kick {
            self.kick(conn, translatable("multiplayer.disconnect.flying", &[]));
        }
        // The vehicle the player drives, by its own gravity: a boat may hover 160 ticks, a horse 80.
        let kick = self.guests.update(i, |g| {
            if g.vehicle_floating && g.vehicle.is_some() {
                g.above_ground_vehicle_ticks += 1;
                if g.above_ground_vehicle_ticks > maximum_flying_ticks(g.vehicle_gravity) {
                    events.push(HostEvent::Warn(format!(
                        "{} was kicked for floating a vehicle too long!",
                        g.name
                    )));
                    return Some(g.conn);
                }
            } else {
                g.vehicle_floating = false;
                g.above_ground_vehicle_ticks = 0;
            }
            None
        });
        if let Some(conn) = kick {
            self.kick(conn, translatable("multiplayer.disconnect.flying", &[]));
        }
    }
}
