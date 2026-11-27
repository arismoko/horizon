use crate::game::ServerGame;
use sunrise_gameplay_common::game::item_abilities::*;

impl ServerGame<'_> {
    /// Whether the held stack shears a mob (`shears_harvest`): shears, or a stack whose
    /// item parts let it.
    pub(in crate::game) fn held_shears(&self, ability: &str) -> bool {
        self.common
            .held()
            .is_some_and(|stack| self.stack_shears(stack, ability))
    }
}
