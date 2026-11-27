//! Placing the local player in a world it opened: at its saved position once the chunks around it
//! are lit, or at the world spawn, then capturing the mouse.
use super::*;

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn spawn_player(&mut self) -> Result<()> {
        if self.common.player.is_some() || !self.common.is_logical_server() {
            return Ok(());
        }
        let saved = self
            .common
            .level
            .position
            .map(|p| Vec3::new(p[0], p[1], p[2]));
        if self.common.spawn.is_none() {
            self.common.spawn = self
                .common
                .level
                .spawn
                .map(|s| Vec3::new(s[0] + 0.5, s[1], s[2] + 0.5));
        }
        let feet = if let Some(p) = saved {
            let (cx, cz) = ((p.x.floor() as i32) >> 4, (p.z.floor() as i32) >> 4);
            self.server.streaming.center = (cx, cz);
            if !(-1..=1).all(|dz| (-1..=1).all(|dx| self.server.streaming.is_lit(cx + dx, cz + dz)))
            {
                return Ok(());
            }
            self.common.level.position = None;
            p
        } else if self.common.spawn.is_none() || self.server.player_spawn.is_some() {
            // A new world: its spawn, then the first player around it.
            if self.common.spawn.is_none() && !self.try_initialize_world_spawn(false)? {
                return Ok(());
            }
            let s = self
                .common
                .level
                .spawn
                .unwrap_or_default()
                .map(|v| v.floor() as i32);
            let Some(feet) = self.find_player_spawn(s)? else {
                return Ok(());
            };
            feet
        } else {
            let s = self.common.spawn.unwrap_or_default();
            let (cx, cz) = ((s.x.floor() as i32) >> 4, (s.z.floor() as i32) >> 4);
            self.server.streaming.center = (cx, cz);
            if !(-1..=1).all(|dz| (-1..=1).all(|dx| self.server.streaming.is_lit(cx + dx, cz + dz)))
            {
                return Ok(());
            }
            let column = [s.x.floor() as i32, s.y.floor() as i32, s.z.floor() as i32];
            if self.common.level.world_spawn == Some(column) {
                // TeleportTransition.missingRespawnBlock: PlayerSpawnFinder
                // around the world spawn, not its exact column; its candidates'
                // chunks load first.
                let Some(feet) = self.find_player_spawn(column)? else {
                    return Ok(());
                };
                feet
            } else {
                // A spawnpoint: the stored column, lifted onto its current surface.
                let y = self
                    .common
                    .land_column(column[0], column[2])
                    .map_or(s.y, f64::from);
                Vec3::new(s.x, y.max(s.y), s.z)
            }
        };
        let mut player = PlayerMotion::new(self.common.motion, feet)?;
        player.yaw = self.common.yaw;
        player.load_saved_state_once(&mut self.server.restore_player_state, &self.common.level)?;
        let spectator = self.common.mode == sunrise_command::args::GameMode::Spectator;
        player.may_fly = self.common.creative || spectator;
        player.invulnerable = self.common.creative || spectator;
        player.spectator = spectator;
        player.local_spectator = spectator;
        if spectator {
            player.flying = true;
        } else if !player.may_fly {
            player.flying = false;
        }
        player.respiration = self.common.armor[0]
            .as_ref()
            .map_or(0, |s| s.enchantment_level("minecraft:respiration") as i32);
        player.pitch = self.common.pitch;
        player.glider = self.common.glider_worn();
        self.common.previous_position = feet;
        self.common.hook.discard();
        self.common.player_state.attachments.retire();
        self.common.resident_entities.local_player.renew();
        self.common.player = Some(player);
        self.bind_local_player_residency();
        self.apply_equipment();
        self.common.eye = 1.62;
        self.common.clock.reset();
        Ok(())
    }
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
