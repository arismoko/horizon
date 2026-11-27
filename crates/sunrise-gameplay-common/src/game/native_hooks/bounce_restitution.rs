use super::*;
use crate::game::native_surface::FrictionView;

impl FrictionView<'_> {
    /// `IBlockExtension.getBounceRestitution`: entity `entity` of type `kind` landed on `state`
    /// at `pos` (`Entity.getOnPosLegacy`). `factor` is the block's own restitution, which its
    /// parts may change. Asked on both sides, in the level of the side that ticks the entity.
    pub(in crate::game) fn native_bounce_restitution(
        &self,
        entity: i32,
        kind: &str,
        state: BlockStateId,
        pos: BlockPos,
        factor: &mut f32,
    ) {
        self.fire_bounce_restitution(entity, kind, state, pos, factor);
    }
}

pub(in crate::game) struct StepOnView<'a>(pub(in crate::game) mod_api::LevelRandomSide<'a>);

impl StepOnView<'_> {
    /// `Block.stepOn` before the simulation checks the blocks it traversed.
    pub(in crate::game) fn native_step_on(
        &mut self,
        step: sunrise_sim::block_contact::StepContact<'_>,
        movement: &mut [f64; 3],
    ) -> Vec<mod_api::EntityWrite> {
        use crate::game::api_values::{TryIntoApi, TryIntoEngine};
        let c = step.contact;
        let Ok(state) = c.state.try_into_api() else {
            return Vec::new();
        };
        let (side, mut queued) = match &mut self.0 {
            mod_api::LevelRandomSide::Server(level) => (
                mod_api::LevelRandomSide::Server(*level),
                mod_api::StepOnWrites::server(),
            ),
            mod_api::LevelRandomSide::Client(level) => (
                mod_api::LevelRandomSide::Client(*level),
                mod_api::StepOnWrites::default(),
            ),
        };
        let mut blocks = Vec::new();
        let mut entity_writes = Vec::new();
        let mut composed = *movement;
        mod_api::dispatch_step_on(
            side,
            c.entity,
            c.kind,
            state,
            mod_api::BlockPos::new(c.position.x, c.position.y, c.position.z),
            step.carefully,
            [
                step.bounds.min_position().components(),
                step.bounds.max_position().components(),
            ],
            &mut queued,
            movement,
            |part, level, event| {
                let previous = *event.movement;
                part.step_on(level, event);
                let (requests, writes) = event.__take_writes();
                // Entity writes drain in order after the callbacks' direct velocity edits.
                // Reuse the composed prefix unless this Part changed the starting velocity.
                let edited = *event.movement != previous;
                let mut velocity = if edited { *event.movement } else { composed };
                let mut accepts = |write: &mod_api::EntityWrite| match *write {
                    mod_api::EntityWrite::Push(v) => {
                        for (axis, push) in velocity.iter_mut().zip(v) {
                            *axis += push;
                        }
                        velocity.iter().all(|v| v.is_finite())
                    }
                    mod_api::EntityWrite::SetMovement(v) => {
                        velocity = v;
                        v.iter().all(|v| v.is_finite())
                    }
                    mod_api::EntityWrite::StuckIn(v) => v.iter().all(|v| v.is_finite()),
                    mod_api::EntityWrite::Hurt { amount, .. } => amount.is_finite(),
                    _ => true,
                };
                let valid = event.movement.iter().all(|v| v.is_finite())
                    && (!edited || entity_writes.iter().all(&mut accepts))
                    && writes.iter().all(accepts);
                let kept = blocks.len();
                if valid
                    && requests
                        .into_iter()
                        .map(|write| write.try_into_engine(&()))
                        .try_for_each(|write| write.map(|write| blocks.push(write)))
                        .is_ok()
                {
                    composed = velocity;
                    entity_writes.extend(writes);
                } else {
                    blocks.truncate(kept);
                    *event.movement = previous;
                }
            },
        );
        sunrise_sim::block_contact::landed(c.state, c.position, blocks);
        entity_writes
    }
}

/// The restitution the simulation's own code (client-lib) asks of a block an entity lands on.
const RESTITUTION_HOOKS: sunrise_sim::block_contact::RestitutionHooks =
    sunrise_sim::block_contact::RestitutionHooks {
        listens: |state| {
            crate::game::api_values::block_state::block_listens(
                mod_api::HookId::BounceRestitution,
                state,
            )
        },
        restitution: |level, c, factor| {
            FrictionView(level, None)
                .native_bounce_restitution(c.entity, c.kind, c.state, c.position, factor);
        },
    };

/// Bind the block contacts and the bounce restitution the simulation reports to the block
/// parts; the first binding stays.
pub fn install_block_contact_hooks() {
    let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
    let _ = sunrise_sim::block_contact::RESTITUTION_HOOKS.set(RESTITUTION_HOOKS);
}
