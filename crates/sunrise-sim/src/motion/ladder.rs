//! `LivingEntity.onClimbable` for the player: the block it stands in, and the mods' say on whether
//! that block is a ladder (`native_is_ladder`, in `motion`) and on whether the block under an open
//! trapdoor makes it one (`native_makes_open_trapdoor_above_climbable`). A sneaking player's slide
//! down it is stopped unless the block is scaffolding, which the mods may also say
//! (`native_is_scaffolding`, in `motion::surface`).
use super::*;
use sunrise_world::living_air;

/// Whether an open trapdoor above a block climbs like a ladder
/// (`IBlockExtension.makesOpenTrapdoorAboveClimbable`, which `LivingEntity.onClimbable` asks of
/// the block under the open trapdoor an entity stands in) is being decided. `state` is that
/// block's state at `pos` and `trapdoor` the trapdoor's state; `climbable` starts with vanilla's
/// answer, a `minecraft:ladder` facing the trapdoor's way, and the block's parts may change it.
/// Fires on replayed ticks too, so a listener must be a pure function of its inputs.
#[inline(never)]
pub fn native_makes_open_trapdoor_above_climbable(
    context: &LoopContext,
    state: BlockStateId,
    pos: BlockPos,
    trapdoor: BlockStateId,
    climbable: &mut bool,
) {
    crate::native_client_hooks::makes_open_trapdoor_above_climbable(
        context, state, pos, trapdoor, climbable,
    );
}

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
    fn adjust_trapdoor_ladder(
        &mut self,
        position: BlockPos,
        state: BlockStateId,
        trapdoor: BlockStateId,
        ladder: bool,
    ) -> bool {
        let mut climbable = ladder;
        native_makes_open_trapdoor_above_climbable(
            self.loop_context,
            state,
            position,
            trapdoor,
            &mut climbable,
        );
        climbable
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
        let blocks = &self.resources.blocks;
        let own = blocks.get(state)?.coefficients.friction;
        let level = FrictionLevel {
            world: self.scene,
            registry: self.assets.registry,
            context: self.loop_context,
            side: self.side,
            coefficient: &|id| Some(blocks.get(id).ok()?.coefficients.friction),
        };
        let player = (self.player.entity_id as u32, "minecraft:player");
        Ok(read_friction(level, Some(player), p, state, own).unwrap_or(own))
    }
    fn friction_modifier(&mut self) -> Result<f64> {
        self.player.attribute("minecraft:friction_modifier")
    }
    fn speed(&mut self) -> Result<f32> {
        let mut speed = self.player.attribute("minecraft:movement_speed")? as f32;
        let side = self.side;
        let reads = || side.hit_reads();
        let living = self.player_lend(&reads);
        native_living_speed(self.loop_context, living, &mut speed);
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
        let side = self.side;
        let reads = || side.hit_reads();
        let living = self.player_lend(&reads);
        native_living_flying_speed(self.loop_context, living, &mut speed);
        Ok(speed)
    }
    fn yaw(&mut self) -> Result<f32> {
        Ok(self.player.yaw)
    }
    fn velocity(&self) -> Vec3 {
        self.player.velocity
    }
    fn set_velocity(&mut self, v: Vec3) -> Result<()> {
        self.player.set_velocity(v);
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
        let position = BlockPos::containing(self.player.position);
        let state = self.block(position);
        let mut scaffolding = self.collision_binding.is_scaffolding(state);
        super::surface::native_is_scaffolding(
            self.loop_context,
            self.player.entity_id as u32,
            position,
            state,
            &mut scaffolding,
        );
        Ok(scaffolding)
    }
    fn suppressing_ladder_slide(&mut self) -> Result<bool> {
        Ok(self.player.shift_key_down && !(self.player.client_side && self.player.flying))
    }
    fn is_player_class(&self) -> bool {
        true
    }
    fn move_self(&mut self, gate: &mut MoveGate, v: Vec3) -> Result<living_air::SelfMove> {
        let before = (self.player.position, self.player.ground.on_ground);
        MotionWorld::move_self(self, gate, v)?;
        Ok(living_air::SelfMove {
            moved: self.player.position.subtract(before.0),
            on_ground_before: before.1,
        })
    }
    /// `living_travel_moved` for the player: the server's, and this client's own prediction,
    /// replayed ticks included.
    fn travel_moved(
        &mut self,
        mode: sunrise_world::travel::TravelMode,
        made: living_air::SelfMove,
    ) -> Result<()> {
        use sunrise_world::travel::TravelMode;
        let flying = self.player.flying;
        let mut velocity = self.player.velocity;
        crate::travel_gate::moved_controlled(
            self.loop_context,
            self.player.entity_id as u32,
            "minecraft:player",
            || match mode {
                TravelMode::Air if flying => TravelMode::Flying,
                other => other,
            },
            crate::travel_gate::Moved {
                moved: made.moved,
                on_ground_before: made.on_ground_before,
                on_ground: self.player.ground.on_ground,
                jumping: self.player.jumping,
            },
            &mut velocity,
            true,
        );
        if velocity != self.player.velocity {
            self.player.set_velocity(velocity);
        }
        Ok(())
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
    /// `Level.hasChunkAt`: a client's level always has one (`ClientLevel.hasChunk` is `true`; a
    /// chunk that hasn't come is the empty chunk), so only a server's looks at its chunks.
    fn has_chunk_at(&mut self, p: BlockPos) -> Result<bool> {
        Ok(self.player.client_side || self.scene.chunks.contains_key(&(p.x >> 4, p.z >> 4)))
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
