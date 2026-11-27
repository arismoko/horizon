//! Read-only liquid-container decision.

pub trait Hooks {
    /// `LiquidBlockContainer.canPlaceLiquid`: may the block at `pos` take `fluid`?
    /// The first answering `BlockPart` returns `Some(true)` to accept as a
    /// container on any template, or `Some(false)` to refuse as a container.
    /// `None` leaves it to the next part, then to the template. A bucket may
    /// still replace a replaceable block, or try the block beside the hit.
    ///
    /// A player's bucket lends that player as `user`, including during client
    /// prediction. The fluid utility lends its optional player; dispensers and
    /// spreading fluid lend `None`. The host is
    /// read-only on both sides. Answer from its reads, not from side effects.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "both",
        contract = "replace",
        scope = "block",
        key = "state",
        out = "answer",
        target = "sunrise_gameplay_common::game::native_hooks::place_liquid::GameLevel<'_>::native_can_place_liquid",
        neoforge = "none (vanilla LiquidBlockContainer.canPlaceLiquid)"
    )]
    fn can_place_liquid(
        pos: BlockPos,
        state: BlockState,
        fluid: mod_api::FluidKey,
        user: Option<mod_api::LivingRef>,
        answer: &mut Option<bool>,
    );
}
