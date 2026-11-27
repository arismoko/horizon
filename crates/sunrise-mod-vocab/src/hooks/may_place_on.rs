//! The `may_place_on` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `VegetationBlock.mayPlaceOn` for the plant `state` on `ground` at
    /// `ground_pos`. The plant inherits vegetation rules from its template or
    /// opts into them in its block declaration. Only when the soil's
    /// `can_sustain_plant` leaves the decision to the plant does this method
    /// answer what it can stand on. Answer only for your own blocks. The level
    /// is read-only.
    /// Asked on both sides, as vanilla asks it: a client asks it from its
    /// placement prediction and its own shape updates, with its loaded copy of
    /// the level (match `LevelSide`). Reads a client can't answer, tag tests
    /// among them, are `None` there, and `BlockEntityPartState::state_in` gives
    /// its default: never answer from those as if they were the server's.
    /// Answer `None` on a client where only a server can decide: the plant's
    /// default rule then predicts, and the server still decides.
    /// Version 2 is catalog metadata; rebuild for the `LevelSide` signature.
    /// A disabled owner's subscribed rule preserves a plant of this type already
    /// at the position or settling there, but does not authorize a new planting.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "both",
        version = 2,
        contract = "replace",
        out = "may",
        scope = "block",
        inert = "signal",
        key = "state",
        target = "sunrise_gameplay_common::game::native_hooks::base::LevelView<'_>::native_may_place_on",
        neoforge = "IBlockExtension.canSustainPlant"
    )]
    fn may_place_on(
        dimension: LevelDimension,
        state: BlockState,
        ground: BlockState,
        ground_pos: BlockPos,
        may: &mut Option<bool>,
        #[engine] inert: &mut bool,
    );
}
