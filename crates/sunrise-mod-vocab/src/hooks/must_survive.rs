//! The block item's placement survival requirement.

pub trait Hooks {
    /// `BlockItem.mustSurvive`: whether placing this item must pass the block's
    /// `can_survive` predicate in the final `BlockItem.canPlace` check. Asked
    /// on both sides after placement state selection, with a read-only level
    /// host. Standing/wall items select candidates using unconditional survival
    /// checks and their own placement-state override; this hook does not skip them.
    /// `Some(false)` skips survival only, not replaceability, bounds or entity
    /// obstruction. `Some(true)` requires survival. `None` keeps vanilla's
    /// requirement (false for scaffolding, true for other block items).
    /// Dispatch is by `item` id; this event has no held-stack components.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "both",
        contract = "replace",
        scope = "item",
        key = "item",
        out = "answer",
        target = "sunrise_gameplay_common::game::native_hooks::must_survive::GameLevel<'_>::native_must_survive",
        neoforge = "none (vanilla BlockItem.mustSurvive)"
    )]
    fn must_survive(item: &str, answer: &mut Option<bool>);
}
