//! The player's read-only block contact Parts.
use super::{FrictionLevel, MotionWorld, Result};
use crate::block_contact;
use sunrise_core::position::BlockPos;
use sunrise_world::registry::BlockStateId;

impl MotionWorld<'_, '_, '_> {
    /// The state and position are the pair selected in Entity.move before its fall check.
    pub(super) fn part_bounce_restitution(
        &self,
        state: BlockStateId,
        position: BlockPos,
    ) -> Result<f32> {
        let own = self
            .resources
            .blocks
            .get(state)?
            .coefficients
            .bounce_restitution;
        Ok(block_contact::bounce_restitution(
            FrictionLevel {
                world: self.scene,
                registry: self.assets.registry,
                context: self.loop_context,
                side: self.side,
                coefficient: &|id| {
                    self.resources
                        .blocks
                        .get(id)
                        .ok()
                        .map(|p| p.coefficients.friction)
                },
            },
            block_contact::Contact {
                entity: self.player.entity_id,
                kind: "minecraft:player",
                state,
                position,
            },
            own,
        ))
    }

    pub(super) fn part_step_on(&mut self, state: BlockStateId, position: BlockPos) -> Result<()> {
        use block_contact::{StepContact, StepLevel};
        if !block_contact::step_listens(state) {
            return Ok(());
        }
        let step = StepContact {
            contact: block_contact::Contact {
                entity: self.player.entity_id,
                kind: "minecraft:player",
                state,
                position,
            },
            carefully: self.player.shift_key_down,
            bounds: self.player.bounds(),
        };
        let blocks = &self.resources.blocks;
        let mut damage;
        let level =
            if self.loop_context.side() == sunrise_world::native_load_context::LoopSide::Server {
                damage = crate::health::DamageLevel::of(
                    self.scene,
                    self.assets.registry,
                    self.tag_items,
                    self.block_registries,
                    self.damage_hooks,
                );
                StepLevel::Server(&mut damage)
            } else {
                StepLevel::Client(FrictionLevel {
                    world: self.scene,
                    registry: self.assets.registry,
                    context: self.loop_context,
                    side: self.side,
                    coefficient: &|id| Some(blocks.get(id).ok()?.coefficients.friction),
                })
            };
        for write in block_contact::step_on(level, step, &mut self.player.velocity) {
            if self.player.health.dead_or_dying() {
                break;
            }
            self.apply_entity_write(write)?;
        }
        Ok(())
    }
}
