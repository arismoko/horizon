//! `LivingEntity.onClimbable` for the player: the block it stands in, and the mods' say on whether
//! that block is a ladder (`native_is_ladder`, in `motion`).
use super::*;

impl ClimbingWorld for MotionWorld<'_, '_, '_> {
    type Error = std::convert::Infallible;
    fn is_spectator(&mut self) -> std::result::Result<bool, Self::Error> {
        Ok(self.player.spectator)
    }
    fn block_position(&mut self) -> std::result::Result<BlockPos, Self::Error> {
        Ok(BlockPos::containing(self.player.position))
    }
    fn in_block_state(&mut self) -> std::result::Result<BlockStateId, Self::Error> {
        Ok(self.block(BlockPos::containing(self.player.position)))
    }
    fn is_fall_flying(&mut self) -> std::result::Result<bool, Self::Error> {
        Ok(self.player.fall_flying)
    }
    fn block_state(
        &mut self,
        position: BlockPos,
    ) -> std::result::Result<BlockStateId, Self::Error> {
        Ok(self.block(position))
    }
    fn set_last_climbable_position(&mut self, position: BlockPos) {
        self.player.last_climbable = Some(position);
    }
    fn adjust_ladder(&mut self, position: BlockPos, state: BlockStateId, tagged: bool) -> bool {
        let mut ladder = tagged;
        native_is_ladder(
            self.loop_context,
            self.player.entity_id as u32,
            position,
            state,
            &mut ladder,
        );
        ladder
    }
}

impl LivingAirWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn movement_support_position(&mut self) -> Result<BlockPos> {
        self.support(movement_support::MOVEMENT_SUPPORT_OFFSET)
    }
    fn on_ground(&mut self) -> Result<bool> {
        Ok(self.player.ground.on_ground)
    }
    fn block_friction(&mut self, p: BlockPos) -> Result<f32> {
        let state = self.block(p);
        let mut friction = self.resources.blocks.get(state)?.coefficients.friction;
        let (context, id) = (self.loop_context, self.player.entity_id as u32);
        native_friction(context, id, "minecraft:player", p, state, &mut friction);
        Ok(friction)
    }
    fn friction_modifier(&mut self) -> Result<f64> {
        self.player.attribute("minecraft:friction_modifier")
    }
    fn speed(&mut self) -> Result<f32> {
        let mut speed = self.player.attribute("minecraft:movement_speed")? as f32;
        native_living_speed(
            self.loop_context,
            self.player.entity_id as u32,
            "minecraft:player",
            &mut speed,
        );
        Ok(speed)
    }
    fn flying_speed(&mut self) -> Result<f32> {
        let mut speed = if self.player.flying {
            // Player.getFlyingSpeed: abilities speed, doubled when sprinting.
            self.player.flying_speed * if self.player.sprinting { 2.0 } else { 1.0 }
        } else if self.player.sprinting {
            0.025999999
        } else {
            0.02
        };
        native_living_flying_speed(
            self.loop_context,
            self.player.entity_id as u32,
            "minecraft:player",
            &mut speed,
        );
        Ok(speed)
    }
    fn yaw(&mut self) -> Result<f32> {
        Ok(self.player.yaw)
    }
    fn velocity(&self) -> Vec3 {
        self.player.velocity
    }
    fn set_velocity(&mut self, v: Vec3) -> Result<()> {
        self.player.velocity = v;
        Ok(())
    }
    fn on_climbable(&mut self) -> Result<bool> {
        self.climbable()
    }
    fn reset_fall_distance(&mut self) -> Result<()> {
        self.player.fall_distance = 0.0;
        Ok(())
    }
    fn in_scaffolding(&mut self) -> Result<bool> {
        Ok(self
            .assets
            .registry
            .state(self.block(BlockPos::containing(self.player.position)))
            .unwrap()
            .name
            == "minecraft:scaffolding")
    }
    fn suppressing_ladder_slide(&mut self) -> Result<bool> {
        Ok(self.player.shift_key_down && !(self.player.client_side && self.player.flying))
    }
    fn is_player_class(&self) -> bool {
        true
    }
    fn move_self(&mut self, gate: &mut MoveGate, v: Vec3) -> Result<()> {
        MotionWorld::move_self(self, gate, v)
    }
    fn horizontal_collision(&self) -> bool {
        self.player.collision.horizontal
    }
    fn jumping(&self) -> bool {
        self.player.jumping
    }
    fn was_in_powder_snow(&self) -> bool {
        self.player.was_in_powder_snow
    }
    fn can_walk_powder_snow(&mut self) -> Result<bool> {
        Ok(self.player.leather_boots)
    }
    fn levitation_amplifier(&mut self) -> Result<Option<i32>> {
        Ok(self.player.status.levitation)
    }
    fn is_client_side(&mut self) -> Result<bool> {
        Ok(self.player.client_side)
    }
    fn has_chunk_at(&mut self, p: BlockPos) -> Result<bool> {
        Ok(self.scene.chunks.contains_key(&(p.x >> 4, p.z >> 4)))
    }
    fn effective_gravity(&mut self) -> Result<f64> {
        living_motion::effective_gravity(self)
    }
    fn y(&self) -> f64 {
        self.player.position.y
    }
    fn minimum_y(&mut self) -> Result<i32> {
        Ok(self.scene.shape.min_y)
    }
    fn discard_friction(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn air_drag_modifier(&mut self) -> Result<f64> {
        self.player.attribute("minecraft:air_drag_modifier")
    }
    fn omnidirectional_air_mover(&mut self) -> Result<bool> {
        Ok(false)
    }
}
