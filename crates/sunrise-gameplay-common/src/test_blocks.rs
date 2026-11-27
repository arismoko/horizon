//! The test block signal rule.
use sunrise_world::registry::{BlockRegistry, BlockStateId};

/// `TestBlock.ownSignal`: the block state's mode is the gate, and only then
/// does the block entity's powered flag count.
pub fn own_signal(registry: &BlockRegistry, state: BlockStateId, powered: bool) -> i32 {
    let start = registry
        .state(state)
        .and_then(|s| s.properties.get("mode"))
        .is_some_and(|mode| mode == "start");
    if start && powered { 15 } else { 0 }
}
