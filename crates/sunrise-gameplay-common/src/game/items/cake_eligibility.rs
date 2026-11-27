use super::*;

pub trait CakeEligibilityGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// Decide in the level, ask with the full read-only game host, then apply.
    /// A cake probe stops at `canEat`, before any effect. Other interactions
    /// finish on the first pass and are never repeated.
    fn with_block_use_level<R>(
        &mut self,
        mut use_block: impl FnMut(&Behaviours, &mut GameLevel<'_>, &mut Ticks) -> R,
    ) -> R {
        if !mod_api::__private::listened(mod_api::HookId::CanEat) {
            return self.with_level(use_block);
        }
        let (result, requested) = self.with_level(|b, level, ticks| {
            level.cake_probe = true;
            let result = use_block(b, level, ticks);
            (result, level.cake_requested.get())
        });
        if !requested {
            return result;
        }
        // The hook has a shared Host: it cannot write game state between the
        // decision and apply. The player's vanilla hunger is unchanged.
        let answer = self.can_eat(None, false);
        self.with_level(|b, level, ticks| {
            level.cake_answer = Some(answer);
            use_block(b, level, ticks)
        })
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> CakeEligibilityGameExt1<'__sg> for G {}

use crate::game::held_items::HeldItemsGameExt1 as _;
