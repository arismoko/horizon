use super::BlockStateId;
use mod_api::{__private::has_scoped_hook, HookId};

impl crate::game::ClientGame<'_> {
    /// `ask` for each of the first `states` block states and each fluid index below `fluids`,
    /// when a part subscribes to `hook`: the answer for (`state`, `fluid`) is at
    /// `state * fluids + fluid`. `None` when no part subscribes, and nothing is asked.
    pub(in crate::game::client_side) fn resolve_per_state_fluid<T>(
        hook: HookId,
        states: usize,
        fluids: usize,
        mut ask: impl FnMut(BlockStateId, usize) -> T,
    ) -> Option<Vec<T>> {
        has_scoped_hook(hook).then(|| {
            (0..states)
                .flat_map(|state| (0..fluids).map(move |fluid| (state, fluid)))
                .map(|(state, fluid)| ask(BlockStateId(state as u32), fluid))
                .collect()
        })
    }
}
