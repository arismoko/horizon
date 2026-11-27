//! A spectating guest's camera (`ServerPlayer.camera`): chosen by a click
//! (`handleSpectatorAction`) or `/spectate`, followed every tick, dropped
//! when the entity dies, the player sneaks, is teleported or leaves
//! spectator mode.
use super::commands::Target;
use sunrise_core::geometry::{Aabb, Vec3};
pub(in crate::game) use sunrise_gameplay::lan::guest_rules::{
    CameraExtra, GuestCamera, camera_ends,
};
use sunrise_protocol::packets::play::{self, Clientbound as C};

impl crate::game::ServerGame<'_> {
    /// `ServerPlayer.isAlive` of the camera, with where it is and how it
    /// looks (`absSnapTo`'s arguments): a player, the host or any other
    /// entity of the level, by network id.
    fn camera_state(&self, id: i32) -> Option<([f64; 3], [f32; 2])> {
        let host = self.server.lan.as_ref()?;
        if let Some(g) = host
            .hub
            .guests
            .iter()
            .find(|g| g.entity_id == id && host.hub.here(g))
        {
            return (!g.dead).then_some((g.position, [g.yaw, g.pitch]));
        }
        if let Some(special) = self.camera_extra(id) {
            return Some((special.position, special.rotation));
        }
        let t = self.level_entity(id)?;
        if let Target::Mob(m) = t
            && !self.common.mobs.iter().any(|x| x.id == m && x.alive())
        {
            return None;
        }
        Some((self.target_position(t)?, self.target_rotation(t)))
    }

    /// The camera targets `Target` does not name: primed TNT and falling
    /// blocks (`isPickable` is `!isRemoved`) and `EnderDragonPart`s
    /// (`ServerLevel.getEntityOrPart`; `isPickable` is `true`, and a part
    /// never turns).
    pub(in crate::game::server_side) fn camera_extra(&self, id: i32) -> Option<CameraExtra> {
        let id = u32::try_from(id).ok().filter(|&id| id != 0)?;
        let at = |v: Vec3| [v.x, v.y, v.z];
        if let Some(t) = self.common.tnts.iter().find(|t| t.id == id) {
            return Some(CameraExtra {
                position: at(t.position),
                rotation: [0.0; 2],
                bounds: t.bounds(),
            });
        }
        if let Some(f) = self.common.falling.iter().find(|f| f.id == id) {
            // EntityType.FALLING_BLOCK: 0.98 x 0.98.
            let h = 0.49;
            return Some(CameraExtra {
                position: at(f.position),
                rotation: [0.0; 2],
                bounds: Aabb::new(
                    Vec3::new(f.position.x - h, f.position.y, f.position.z - h),
                    Vec3::new(f.position.x + h, f.position.y + 0.98, f.position.z + h),
                ),
            });
        }
        let (position, bounds) = self.dragon_part(id)?;
        Some(CameraExtra {
            position: at(position),
            rotation: [0.0; 2],
            bounds,
        })
    }

    /// `Entity.isPickable` of the entity with that id.
    fn camera_pickable(&self, id: i32) -> bool {
        let Some(host) = self.server.lan.as_ref() else {
            return false;
        };
        // Player.isPickable: not a spectator.
        if let Some(g) = host.hub.guests.iter().find(|g| g.entity_id == id) {
            return !g.spectator() && !g.dead;
        }
        if self.camera_extra(id).is_some() {
            return true;
        }
        match self.level_entity(id) {
            // Projectile.isPickable: the redirectable projectiles
            // (`#redirectable_projectile`); ShulkerBullet's is `true`.
            Some(t @ Target::Projectile(_)) => matches!(
                self.target_type(t).as_str(),
                "minecraft:fireball"
                    | "minecraft:wind_charge"
                    | "minecraft:breeze_wind_charge"
                    | "minecraft:shulker_bullet"
            ),
            // LivingEntity: not removed. ArmorStand: not a marker.
            Some(Target::Mob(_)) => self.camera_state(id).is_some(),
            Some(Target::Stand(s)) => self
                .common
                .armor_stands
                .iter()
                .any(|a| a.id == s && !a.marker),
            // BlockAttachedEntity, minecarts, boats, end crystals.
            Some(Target::Frame(_) | Target::Painting(_) | Target::Boat(_) | Target::Cart(_))
            | Some(Target::Crystal(_)) => true,
            // Interaction.
            Some(t @ Target::Other(_)) => self.target_type(t) == "minecraft:interaction",
            _ => false,
        }
    }

    /// `ServerGamePacketListenerImpl.handleSpectatorAction`.
    pub(in crate::game::server_side) fn guest_spectator_action(
        &mut self,
        gi: usize,
        target: Option<i32>,
    ) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let g = &host.hub.guests[gi];
        if !g.loaded || !g.spectator() {
            return;
        }
        let conn = g.conn;
        let Some(id) = target else {
            return;
        };
        let Some(bounds) = self
            .guest_target_box(gi, id)
            .or_else(|| self.camera_extra(id).map(|e| e.bounds))
        else {
            return;
        };
        let Some((at, _)) = self.camera_state(id) else {
            return;
        };
        let block = at.map(|v| v.floor() as i32);
        if !self
            .common
            .border
            .contains(f64::from(block[0]), f64::from(block[2]))
        {
            return;
        }
        let in_range = self.as_guest(conn, |g| g.within_entity_interaction_range(&bounds, 3.0));
        if in_range != Some(true) {
            return;
        }
        if self.camera_pickable(id) {
            self.guest_set_camera(gi, Some(id));
        }
    }

    /// `ServerPlayer.setCamera`: the view moves to the new camera, which
    /// the client is told, or back to the player (`None`).
    pub fn guest_set_camera(&mut self, gi: usize, camera: Option<i32>) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let guest = &host.hub.guests[gi];
        let (conn, own, uuid, name) = (guest.conn, guest.entity_id, guest.uuid, guest.name.clone());
        let level = super::Dim::from_id(&guest.dimension);
        let previous = self.server.cmds.0.guest_cameras.get(&conn).map(|c| c.id);
        let camera = camera.filter(|id| *id != own);
        if self.server.cmds.0.guest_cameras.get(&conn).map(|c| c.id) == camera {
            return;
        }
        // A target with no state (gone, dead, in another level) is no camera: nothing changes
        // and nobody is told.
        let at = match camera {
            Some(id) => match self.camera_state(id) {
                Some((position, _)) => position,
                None => return,
            },
            None => guest.position,
        };
        let back = self.common.dim;
        if level != back
            && let Err(error) = self.enter_level(level)
        {
            log::error!("could not reopen {}: {error}", level.id());
            return;
        }
        match camera {
            Some(id) => {
                self.server
                    .cmds
                    .0
                    .guest_cameras
                    .insert(conn, GuestCamera { id, teleports: 0 });
            }
            None => {
                self.server.cmds.0.guest_cameras.remove(&conn);
            }
        }
        // ServerPlayer.broadcastToPlayer: a spectator with a camera is shown to nobody.
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        {
            let value = camera.is_some();
            host.hub.guests.update(gi, |guest| guest.watching = value);
        }
        // setCamera calls teleportTo before sending the camera packet: the player
        // dismounts, keeps its look, and zeroes its velocity in the same live body.
        self.as_guest(conn, |game| {
            game.place_player_teleport_in_level(Vec3::new(at[0], at[1], at[2]), None, [false; 3])
        });
        if let Some(host) = self.server.lan.as_mut() {
            // setCamera moves the tickets at once: to the camera, or back to the player.
            host.hub.guests.update(gi, |guest| guest.release_tickets());
            host.hub.send_to(
                conn,
                &C::SetCamera(play::SetCamera {
                    camera_id: camera.unwrap_or(own),
                }),
            );
            let teleports = host.hub.guests[gi].teleport_counter;
            if let Some(c) = self.server.cmds.0.guest_cameras.get_mut(&conn) {
                c.teleports = teleports;
            }
            if let Some(entity_id) = camera {
                self.native_player_start_spectating_entity(uuid, &name, entity_id);
            } else if let Some(entity_id) = previous {
                self.native_player_stop_spectating_entity(uuid, &name, entity_id);
            }
        }
        if level != back
            && let Err(error) = self.enter_level(back)
        {
            log::error!("could not reopen {}: {error}", back.id());
        }
    }

    /// Apply `/spectate` camera requests queued by commands and prune disconnected guests.
    pub(in crate::game::server_side) fn apply_guest_camera_requests(&mut self) {
        for (conn, id) in std::mem::take(&mut self.server.cmds.0.camera_requests) {
            if let Some(gi) = self
                .server
                .lan
                .as_ref()
                .and_then(|host| host.hub.guests.iter().position(|g| g.conn == conn))
            {
                self.guest_set_camera(gi, id);
            }
        }
        if let Some(host) = self.server.lan.as_ref() {
            self.server
                .cmds
                .0
                .guest_cameras
                .retain(|conn, _| host.hub.guests.connection_index(*conn).is_some());
        }
    }

    /// The camera part of `ServerPlayer.tick` for one guest `conn`.
    pub(in crate::game::server_side) fn tick_guest_camera_one(&mut self, conn: u32) {
        let Some(camera) = self.server.cmds.0.guest_cameras.get(&conn).copied() else {
            return;
        };
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let Some(gi) = host.hub.guests.connection_index(conn) else {
            self.server.cmds.0.guest_cameras.remove(&conn);
            return;
        };
        let g = &host.hub.guests[gi];
        let state = self.camera_state(camera.id);
        if camera_ends(
            state.is_some(),
            g.sneaking,
            g.spectator(),
            g.teleport_counter != camera.teleports,
        ) {
            self.guest_set_camera(gi, None);
        } else if let Some((at, [yaw, pitch])) = state {
            // absSnapTo; the level's chunk source follows the player.
            let Some(host) = self.server.lan.as_mut() else {
                return;
            };
            host.hub.guests.update(gi, |g| {
                g.position = at;
                (g.yaw, g.pitch) = (yaw, pitch);
            });
        }
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
