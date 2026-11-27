use super::*;

impl crate::game::CommonGame<'_> {
    // See #3242: fold with resolve_block_paths when it lands
    pub(in crate::game) fn resolve_piston_reactions(&self) {
        let asked = mod_api::__private::has_scoped_hook(mod_api::HookId::PistonPushReaction);
        self.res.behaviours().resolve_piston_reactions(|state| {
            if !asked {
                return None;
            }
            let mut reaction = None;
            crate::game::native_hooks::native_piston_push_reaction(state, &mut reaction);
            reaction
                .as_ref()
                .map(|value| crate::game::api_values::push_reaction(value).1)
        });
    }
}

/// Pure state-only callback, with no game receiver or Host.
pub fn native_piston_push_reaction(
    state: BlockStateId,
    reaction: &mut Option<mod_api::PushReaction>,
) {
    crate::game::native_hooks::fire::fire_piston_push_reaction(state, reaction);
}
