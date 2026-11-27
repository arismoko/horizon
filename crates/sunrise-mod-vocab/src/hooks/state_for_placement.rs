//! The `state_for_placement` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Choose a block's placement candidate (`Block.getStateForPlacement(BlockPlaceContext)`),
    /// with `state` the block's own default state. This is not a placement notice:
    /// code holding a `ServerHost` can call `LevelRef::state_for_placement` to ask
    /// for a candidate without placing anything. The Part still has only its
    /// `LevelHost` through `LevelSide`, not the caller's `ServerHost`.
    /// Answer only for your blocks: put the state you want in `answer` as
    /// `Some(Some(state))`, including a registered state of another block.
    /// If this is an actual placement, survival and entity obstruction use that
    /// candidate at the placement position, but the selected block's
    /// `getStateForPlacement` is not called. A mod selecting a multi-cell block is responsible for
    /// checking its footprint: `setPlacedBy` does not recheck its companion
    /// cell. Sunrise also requires creative-operator permission to select a
    /// game-master block. An unknown state falls back to ordinary placement.
    /// Refuse with `Some(None)`: the block goes nowhere and the stack is kept,
    /// as vanilla's `null`. Left empty, the block's own placement runs. `context` is
    /// vanilla's `BlockPlaceContext` (the clicked face, the clicked position,
    /// `source` distinguishing `PlacementSource::Player { yaw, pitch, sneaking }`
    /// from a non-player `PlacementSource::Directional { direction }`, whether the
    /// clicked block is being replaced, and the item key in hand, not an item stack
    /// or its components). For a directional source, whether the clicked block is
    /// replaced is the game's answer from the clicked block, as
    /// `DirectionalPlaceContext.replacingClickedOnBlock()` is, and a block whose own
    /// rule reads the placer's vertical look (pointed dripstone) has no vanilla answer:
    /// left empty, that placement is refused (`PlacementStateError::NoVerticalLook`
    /// for `LevelRef::state_for_placement`), and answering replaces that rule. `fluid` is
    /// the fluid in the position the block would go in, which a waterlogged block reads. Build the state from `state` with
    /// `native_state_with_property`, and know whether `state` is yours with
    /// `native_state_block`. It is asked for every placer that goes
    /// through vanilla's placement (a player's `BlockItem.place`, a
    /// dispenser's `DirectionalPlaceContext`). The item's `must_survive` controls
    /// the ordinary final survival check. Standing/wall candidate survival and
    /// entity obstruction are independent. A trait mod returns
    /// `Some(Some(state))` to choose a state, `Some(None)` to refuse, or
    /// `None` to leave the answer to the next mod or vanilla. Changes to the event's fluid
    /// copy are mod-provided data forwarded to later Parts, never consumed by the engine.
    /// Asked on both sides, as vanilla asks it: a client asks it from its
    /// placement prediction, with its loaded copy of the level (match
    /// `LevelSide`). Reads a client can't answer, tag tests among them, are
    /// `None` there, and `BlockEntityPartState::state_in` gives its default:
    /// never answer from those as if they were the server's. A client refusal
    /// keeps its stack unspent until the server answers. Answer `None` on a
    /// client where only a server can decide: the template's rule then
    /// predicts, and the server still decides.
    #[notes = r##"### Choosing a block placement state

`BlockPart::state_for_placement` is vanilla's `Block.getStateForPlacement` for
player and dispenser placement, including client prediction for a player.
`event.context.source` distinguishes a player placement
(`PlacementSource::Player { yaw, pitch, sneaking }`) from a non-player
`DirectionalPlaceContext` (`PlacementSource::Directional { direction }`, such as
a dispenser placing a shulker box). A directional context's `replace_clicked` is
read from the clicked block. Where vanilla's directional context throws, as for a
pointed dripstone's vertical look, ordinary placement refuses: answer for your block
to place it anyway.
Code with a `ServerHost` can also call `LevelRef::state_for_placement` to ask
for a candidate without placing anything. The Part still receives only its
`LevelHost` through `LevelSide`; the context carries an item key, not a stack.
Answer only for your own blocks: `Some(Some(state))` selects a registered
candidate, `Some(None)` refuses placement without spending the stack, and
`None` delegates to the next Part or ordinary placement. Use
`native_state_with_property(state, key, value)` to set a property, including
one your block declares.

A candidate can belong to another block. The selected block does not run its
placement-state method again. If actually placed, it still gets the primary
cell's survival and obstruction checks as applicable, and a game-master
block requires creative operator permission. If the selected block spans cells, check its footprint
before returning it: `setPlacedBy` does not recheck its companion cell.
"##]
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "both",
        version = 4,
        contract = "replace",
        out = "answer",
        scope = "block",
        key = "state",
        target = "sunrise_gameplay_common::game::native_hooks::base::LevelView<'_>::native_state_for_placement",
        neoforge = "none (vanilla Block.getStateForPlacement; NeoForge adds no event or extension for it)"
    )]
    fn state_for_placement(
        dimension: LevelDimension,
        state: BlockState,
        context: &PlacementContext,
        fluid: mod_api::FluidState,
        answer: &mut Option<Option<BlockState>>,
    );
}
