//! The block item's pre-placement target rewrite.

pub trait Hooks {
    /// `BlockItem.updatePlacementContext`: replace `context` before selecting
    /// the block's placement state, on both sides with a read-only level host.
    /// `Some(Some(context))` rewrites the target and click, `Some(None)` refuses
    /// placement, and `None` keeps the item's vanilla rewrite (including
    /// scaffolding's column walk). The placing item cannot be changed.
    /// The actual pre-use `stack` supplies the item key and components. `context.source`
    /// distinguishes a player placement (`PlacementSource::Player { yaw, pitch, sneaking }`)
    /// from a directional non-player placement (`PlacementSource::Directional { direction }`):
    /// the effective placement rules, which a rewrite can change independently of `placer`.
    /// `placer` supplies the actual player's handle and eyes on this side, or is absent for a
    /// dispenser. A player's original destination may be occupied: a part can
    /// redirect the placement even when vanilla would refuse it there. A
    /// dispenser still needs a replaceable front cell before asking the part.
    /// With no answer, vanilla checks the original destination before its own
    /// rewrite. A part's rewritten destination must be loaded, in bounds, and
    /// replaceable with the supplied context. Non-finite click or player view
    /// values are refused. A rewritten directional context's `replace_clicked` is
    /// read from the block at `hit_pos`, not from the answer, and a directional
    /// question vanilla throws on (`PlacementStateError::NoVerticalLook` or
    /// `ReplacingClickedCycle`) refuses the placement. Survival and obstruction
    /// still apply at the final position.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "both",
        contract = "replace",
        scope = "item",
        key = "stack",
        out = "answer",
        target = "sunrise_gameplay_common::game::native_hooks::placement_context::GameLevel<'_>::native_placement_context",
        neoforge = "none (vanilla BlockItem.updatePlacementContext)",
        version = 3
    )]
    fn placement_context(
        stack: &ItemStack,
        context: &PlacementContext,
        placer: Option<mod_api::PlacementPlacer>,
        answer: &mut Option<Option<mod_api::PlacementContext>>,
    );
}
