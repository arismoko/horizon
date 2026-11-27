//! The server liquid-container write.

pub trait Hooks {
    /// `LiquidBlockContainer.placeLiquid`, before the template fills `state` at
    /// `pos`. A bucket supplies a source `fluid`; spreading supplies the state
    /// that would flow there. Live structure templates supply the displaced
    /// fluid or a neighbouring source, without asking `can_place_liquid`.
    /// The first `Some(bool)` replaces the template. Return true after storing
    /// the fluid and scheduling any tick it needs, or false when it was not
    /// placed. `None` leaves it to the next part, then to the template. A plain
    /// bucket still empties on false, unlike a validated fluid-container operation.
    ///
    /// Server only. As in vanilla, what a part writes stays written whatever it
    /// answers: player buckets and the fluid utility keep its writes in the outer
    /// transaction (a panicking part's are discarded), and spreading, dispensers
    /// and live template restores write through the direct level write host. If a
    /// transaction is open, those direct paths log and skip the part, leaving the
    /// template's answer. On either host, `container_signal` reads stored
    /// slots without generating deferred loot.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "replace",
        scope = "block",
        key = "state",
        out = "answer",
        version = 2,
        target = "sunrise_gameplay_common::game::native_hooks::place_liquid::GameLevel<'_>::native_place_liquid",
        neoforge = "none (vanilla LiquidBlockContainer.placeLiquid)"
    )]
    fn place_liquid(
        pos: BlockPos,
        state: BlockState,
        fluid: mod_api::FluidState,
        answer: &mut Option<bool>,
        #[engine] ticks: &mut Ticks,
    );
}
