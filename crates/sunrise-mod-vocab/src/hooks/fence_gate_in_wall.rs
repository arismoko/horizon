//! A neighbour's fence-gate wall-height answer.

pub trait Hooks {
    /// Whether `state`, the neighbour of a fence gate, counts as a wall for
    /// that gate's `in_wall` property (`FenceGateBlock.isWall`). Asked of the
    /// neighbour's block part, on placement and horizontal shape updates on
    /// both sides. `Some(true)` lowers the gate, `Some(false)` rejects this
    /// neighbour, and `None` keeps vanilla's `minecraft:walls` tag check.
    /// This does not change wall connections or pathfinding. Answer from
    /// the state alone; the host is read-only.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "both",
        contract = "replace",
        scope = "block",
        key = "state",
        out = "answer",
        target = "sunrise_gameplay_common::game::native_hooks::fence_gate_in_wall::GameLevel<'_>::native_fence_gate_in_wall",
        neoforge = "none (vanilla FenceGateBlock.isWall)"
    )]
    fn fence_gate_in_wall(state: BlockState, answer: &mut Option<bool>);
}
