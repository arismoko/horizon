use super::*;

use sunrise_gameplay_common::game::remote::abilities::walking_speed;

impl Remote {
    /// `AbstractClientPlayer.gameMode()` for a remote player entity in `level_players`.
    pub(in crate::game) fn remote_player_info_game_mode(&self, entity_id: i32) -> Option<i32> {
        if let Some(slot) = self.level_player_infos.get(&entity_id)
            && let Some(cell) = slot.get()
        {
            return Some(cell.load(std::sync::atomic::Ordering::Relaxed));
        }
        let uuid = self.level_players.get(&entity_id)?;
        if let Some(cell) = self.tab_game_modes.get(uuid) {
            if let Some(slot) = self.level_player_infos.get(&entity_id) {
                let _ = slot.set(Arc::clone(cell));
            }
            return Some(cell.load(std::sync::atomic::Ordering::Relaxed));
        }
        self.tab_players.get(uuid).map(|t| t.game_mode)
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `Abilities.getWalkingSpeed()` of the local player: the server's, on a server.
    pub fn remote_walking_speed(&self) -> f32 {
        walking_speed(self.client.remote.as_ref().map(|r| &r.abilities))
    }

    /// Mirror `Remote::abilities` onto the live `PlayerMotion` body.
    pub(in crate::game::client_side) fn apply_remote_abilities(&mut self) {
        let (Some(player), Some(a)) = (
            self.common.player.as_mut(),
            self.client.remote.as_ref().map(|r| &r.abilities),
        ) else {
            return;
        };
        player.invulnerable = a.invulnerable;
        player.flying = a.flying;
        player.may_fly = a.may_fly;
        player.flying_speed = a.flying_speed;
    }
    /// ClientPacketListener.handleRotatePlayer: the server's yaw and pitch, each added to the
    /// current one when its bit says relative, then sent back as a `Rot` move with both
    /// flags false.
    pub(in crate::game) fn remote_rotation(&mut self, p: &play::PlayerRotation) {
        self.common.yaw = if p.relative_yaw {
            self.common.yaw + p.yaw
        } else {
            p.yaw
        };
        self.common.pitch = (if p.relative_pitch {
            self.common.pitch + p.pitch
        } else {
            p.pitch
        })
        .clamp(-90.0, 90.0);
        self.snap_player_model_pitch();
        self.send(S::MovePlayerRot(play::MovePlayerRot {
            yaw: self.common.yaw,
            pitch: self.common.pitch,
            flags: 0,
        }));
    }
    /// Mirror spectator scroll-wheel `flying_speed` back into `Remote::abilities`.
    pub(in crate::game::client_side) fn set_remote_flying_speed(&mut self, speed: f32) {
        if let Some(r) = self.client.remote.as_mut() {
            r.abilities.flying_speed = speed;
        }
    }
    /// Sync local and remote player spectator modes from `PlayerInfo` and `MultiPlayerGameMode`.
    pub(in crate::game::client_side) fn sync_remote_player_game_mode(&mut self) {
        debug_assert!(self.client.remote.is_some());
        if self.common.player.is_some() {
            let info_spectator = self.own_player_is_spectator();
            let local_spectator = self.common.spectator();
            if let Some(player) = self.common.player.as_mut() {
                player.spectator = info_spectator;
                player.local_spectator = local_spectator;
            }
        }
        if let Some(r) = self.client.remote.as_ref() {
            for i in 0..self.common.mobs.len() {
                let id = self.common.mobs[i].id as i32;
                if r.level_players.contains_key(&id) {
                    let spectator = r.remote_player_info_game_mode(id) == Some(SPECTATOR);
                    self.common
                        .mobs
                        .update(i, |m| m.extra.spectator = spectator);
                }
            }
        }
    }
}

impl Remote {
    /// `AbstractClientPlayer.gameMode()`: resolves `PlayerInfo` once per entity lifetime.
    pub(in crate::game::client_side) fn own_player_info_game_mode(&self) -> Option<i32> {
        if let Some(cell) = self.own_player_info.get() {
            return Some(cell.load(std::sync::atomic::Ordering::Relaxed));
        }
        if let Some(cell) = self.tab_game_modes.get(&self.profile) {
            let _ = self.own_player_info.set(Arc::clone(cell));
            return Some(cell.load(std::sync::atomic::Ordering::Relaxed));
        }
        self.tab_players.get(&self.profile).map(|t| t.game_mode)
    }
    /// Copy predicted body flight and scroll speed into `Remote::abilities` before mode updates.
    pub(in crate::game::client_side) fn sync_body_flight(
        &mut self,
        flying: bool,
        flying_speed: f32,
    ) {
        self.abilities.flying = flying;
        self.abilities.flying_speed = flying_speed;
    }
    /// Apply `GameType.updatePlayerAbilities` onto `LocalPlayer.abilities` and mirror `game_mode`.
    pub(in crate::game::client_side) fn update_local_mode_abilities(&mut self, mode: i32) -> bool {
        self.game_mode = mode;
        self.abilities.update_for_mode(mode);
        self.abilities.instabuild
    }
}

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;
