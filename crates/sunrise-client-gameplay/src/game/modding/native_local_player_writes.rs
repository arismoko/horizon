//! A client mod's writes to the player it moves itself, before the tick's own move.
//! Nothing is sent; the server hears of the result in the normal position report.
use crate::game::ClientGame;
use mod_api::{LevelWriteError, PlayerRef};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use sunrise_world::entity_move::MoverType;

impl ClientGame<'_> {
    /// `Entity.setDeltaMovement` on the local player. `false`, with the velocity unchanged,
    /// with no player of its own and for a value that is not finite.
    pub fn set_local_player_velocity(&mut self, velocity: [f64; 3]) -> bool {
        let velocity = Vec3::new(velocity[0], velocity[1], velocity[2]);
        match self.common.player.as_mut() {
            Some(body) if velocity.is_finite() => {
                body.velocity = velocity;
                true
            }
            _ => false,
        }
    }

    /// Sets the local player's fall-flying flag, so the next tick's glide detection sends
    /// `START_FALL_FLYING` as a jump-started glide does. `false` with no player of its own.
    pub fn start_local_player_gliding(&mut self) -> bool {
        match self.common.player.as_mut() {
            Some(body) => {
                body.fall_flying = true;
                true
            }
            None => false,
        }
    }

    /// The local player's on-ground flag, which this tick's jump decision reads and its move
    /// then sets again from the collision. `false` with no player of its own.
    pub fn set_local_player_on_ground(&mut self, on_ground: bool) -> bool {
        match self.common.player.as_mut() {
            Some(body) => {
                body.ground.on_ground = on_ground;
                true
            }
            None => false,
        }
    }

    /// Whether the local player can move itself rather than being held in place.
    pub(in crate::game::client_side) fn can_move_local_player(&self) -> bool {
        self.common.riding.is_none()
            && self.common.sleeping.is_none()
            && self.client.screen != super::Screen::Dead
    }

    pub(in crate::game) fn player_set_body_yaw(
        &mut self,
        player: PlayerRef,
        yaw: f32,
    ) -> Result<bool, LevelWriteError> {
        if self.native_local_status(player).is_none() || self.native_client_level().is_none() {
            return Ok(false);
        }
        let Some(model) = self.client.player_model.as_mut() else {
            return Ok(false);
        };
        model.body_yaw = yaw;
        Ok(true)
    }

    pub(in crate::game) fn player_reset_fall_distance(
        &mut self,
        player: PlayerRef,
    ) -> Result<bool, LevelWriteError> {
        if self.native_local_status(player).is_none() {
            return Ok(false);
        }
        let Some(player) = self.common.player.as_mut() else {
            return Ok(false);
        };
        player.fall_distance = 0.0;
        Ok(true)
    }

    pub fn move_local_player_by(
        &mut self,
        delta: [f64; 3],
    ) -> Result<Option<[f64; 3]>, LevelWriteError> {
        if self.common.player.is_none()
            || !self.can_move_local_player()
            || !sunrise_sim::move_gate::valid_mod_move_delta(delta)
        {
            return Ok(None);
        }
        let [x, y, z] = delta;
        self.client_in_this_levels_colliders(|game| {
            game.move_local_player(MoverType::SelfMovement, Vec3::new(x, y, z), false)
        })
        .map(|moved| moved.map(Vec3::components))
        .map_err(|_| LevelWriteError::Unavailable)
    }
}

use sunrise_gameplay_common::game::piston_push::PistonPushGameExt1 as _;
