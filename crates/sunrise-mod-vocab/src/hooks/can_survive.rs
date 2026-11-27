//! The `can_survive` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `BlockState.canSurvive` for `state` at `pos`
    /// (`BlockBehaviour.canSurvive`), before the block answers it: as it is
    /// placed (`BlockItem.canPlace`), from a shape update of a block whose
    /// template checks it (a plant, a torch, a sign: false breaks the block
    /// with its drops), and wherever else vanilla asks. `state` may not be in
    /// the world yet, so decide from `state`. Answer only for your own
    /// blocks: `Some(false)` refuses the spot, `Some(true)` allows it, and
    /// the block's own rule doesn't run. The level is read-only. Asked on
    /// every shape update of such a block: keep it short.
    /// Asked on both sides, as vanilla asks it: a client asks it from its
    /// placement prediction and its own shape updates, with its loaded copy of
    /// the level (match `LevelSide`). Reads a client can't answer, tag tests
    /// among them, are `None` there, and `BlockEntityPartState::state_in` gives
    /// its default: never answer from those as if they were the server's.
    /// Answer `None` on a client where only a server can decide: the template's
    /// rule then predicts, and the server still decides.
    /// Version 2 is catalog metadata; rebuild for the `LevelSide` signature.
    /// A disabled owner's subscribed rule keeps that block type where it is already present
    /// or settling after a piston move or fall, but refuses a new placement.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "both",
        version = 2,
        contract = "replace",
        out = "survives",
        scope = "block",
        inert = "signal",
        key = "state",
        target = "sunrise_gameplay_common::game::native_hooks::base::LevelView<'_>::native_can_survive",
        neoforge = "none (vanilla BlockBehaviour.canSurvive; NeoForge adds no event or extension for it)"
    )]
    fn can_survive(
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockState,
        survives: &mut Option<bool>,
        #[engine] inert: &mut bool,
    );
}
