use super::BlockStateId;
use mod_api::{__private::has_scoped_hook, HookId};

impl crate::game::ServerGame<'_> {
    /// `ask` for each of the first `states` block states, in id order, when a part subscribes
    /// to `hook`; `None` when none does, and nothing is asked.
    pub(in crate::game) fn resolve_per_state<T>(
        hook: HookId,
        states: usize,
        mut ask: impl FnMut(BlockStateId) -> T,
    ) -> Option<Box<[T]>> {
        has_scoped_hook(hook).then(|| {
            (0..states)
                .map(|state| ask(BlockStateId(state as u32)))
                .collect()
        })
    }
}
