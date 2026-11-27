use sunrise_core::{geometry::Direction, position::BlockPos};
use sunrise_gameplay_common::game::api_values::block_state::block_listens;
use sunrise_world::registry::BlockStateId;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn mod_add_hit_effects(
        &mut self,
        state: BlockStateId,
        pos: [i32; 3],
        face: Direction,
    ) -> Option<bool> {
        if !block_listens(mod_api::HookId::AddHitEffects, state) {
            return None;
        }
        let level = self.native_client_level()?;
        let mut cancel = false;
        self.lend_client_level_random(|game, random| {
            game.native_add_hit_effects(super::fire::AddHitEffectsCall {
                level,
                state,
                pos: BlockPos::new(pos[0], pos[1], pos[2]),
                face,
                random,
                cancel: &mut cancel,
            });
        });
        Some(cancel)
    }

    pub(in crate::game) fn mod_add_destroy_effects(
        &mut self,
        state: BlockStateId,
        pos: [i32; 3],
    ) -> Option<bool> {
        if !block_listens(mod_api::HookId::AddDestroyEffects, state) {
            return None;
        }
        let level = self.native_client_level()?;
        let mut cancel = false;
        self.lend_client_level_random(|game, random| {
            game.native_add_destroy_effects(super::fire::AddDestroyEffectsCall {
                level,
                state,
                pos: BlockPos::new(pos[0], pos[1], pos[2]),
                random,
                cancel: &mut cancel,
            });
        });
        Some(cancel)
    }

    pub(in crate::game) fn native_add_hit_effects(
        &mut self,
        call: super::fire::AddHitEffectsCall<'_>,
    ) {
        self.fire_add_hit_effects(call);
    }

    pub(in crate::game) fn native_add_destroy_effects(
        &mut self,
        call: super::fire::AddDestroyEffectsCall<'_>,
    ) {
        self.fire_add_destroy_effects(call);
    }
}
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
