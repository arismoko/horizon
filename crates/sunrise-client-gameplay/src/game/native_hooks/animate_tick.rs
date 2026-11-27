use sunrise_gameplay_common::game::api_values::block_state::block_listens;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Whether `state`'s block has a part answering `animate_tick`: the per-block table's
    /// answer, no dispatch. The sampler asks it after each loaded pick's block effects.
    pub fn animate_part_listens(state: BlockStateId) -> bool {
        block_listens(mod_api::HookId::AnimateTick, state)
    }
    /// Whether a client tick's sampling asks for the block parts' `animate_tick`: some part
    /// answers it and this is a window's client.
    pub fn mod_animate_parts(&self) -> bool {
        mod_api::__private::has_scoped_hook(mod_api::HookId::AnimateTick)
            && self.common.native_window()
    }

    /// Calls the parts of the pick at `pos` (`state`), inside the pick as vanilla's
    /// `doAnimateTick` calls `Block.animateTick`: after the block's own effects, before its
    /// fluid's and the next pick. `event.random` is the level's random itself: the part's
    /// draws and the game's own during the call (`add_particle`'s limiter) move it in order.
    pub fn mod_animate_tick(&mut self, state: BlockStateId, pos: [i32; 3]) {
        let Some(level) = self.native_client_level() else {
            return;
        };
        self.lend_client_level_random(|game, random| {
            game.native_animate_tick(super::fire::AnimateTickCall {
                level,
                pos: BlockPos::new(pos[0], pos[1], pos[2]),
                state,
                random,
            });
        });
    }

    /// A client's random display tick of the mod block `state` at `pos` (`Block.animateTick`),
    /// picked by `ClientLevel.animateTick`'s sampling: its parts add particles and play sounds
    /// through the client host, drawing from the level's `random`. Client-side.
    pub(in crate::game) fn native_animate_tick(&mut self, call: super::fire::AnimateTickCall<'_>) {
        self.fire_animate_tick(call);
    }
}
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
