//! The `is_valid_bonemeal_target` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A block nothing in Sunrise knows a `BonemealableBlock` for is about to
    /// take bone meal: vanilla's `BonemealableBlock.isValidBonemealTarget`,
    /// `state` the block's own state at `pos`. Answer `valid` for your own
    /// blocks: the meal is used up on one as vanilla uses it up (the growth
    /// particles and the sound follow, whether or not the roll below grows
    /// it), from a player's `BoneMealItem.useOn`, from a dispenser and, on a
    /// crop (a villager bonemeals only a `CropBlock`), from a villager. Asked
    /// on both sides, as vanilla asks it on both, so a client predicts the
    /// use with your answer: say no where you can only grow on a server
    /// (match `LevelSide`). `Some(false)` refuses the meal, even on a block
    /// whose template takes it (vanilla's `growCrop` then keeps the meal);
    /// left `None`, the block keeps the `BonemealableBlock` its template gives
    /// it, or takes no bone meal. A disabled owner's subscribed Part refuses bone meal.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "both",
        contract = "replace",
        out = "valid",
        scope = "block",
        inert = "*valid = Some(false);",
        key = "state",
        target = "sunrise_gameplay_common::game::native_hooks::base::GameLevel<'_>::native_is_valid_bonemeal_target",
        neoforge = "none (vanilla BonemealableBlock.isValidBonemealTarget; NeoForge adds no event or extension for it); BonemealEvent fires when meal is used, before the block's contract"
    )]
    fn is_valid_bonemeal_target(
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockState,
        valid: &mut Option<bool>,
    );
}
