use sunrise_level::containers::{ChestHalf, is_chest_block};

pub fn is_shulker_box(name: &str) -> bool {
    name.ends_with("shulker_box")
}

/// A chest block state's half: its block, `type` and `facing`.
pub fn chest_half(state: &sunrise_world::chunk::BlockState) -> Option<ChestHalf<'_>> {
    Some(ChestHalf {
        block: &state.name,
        kind: state.properties.get("type")?,
        facing: state.properties.get("facing")?,
    })
}
pub fn facing_step(facing: &str) -> (i32, i32) {
    match facing {
        "north" => (0, -1),
        "south" => (0, 1),
        "west" => (-1, 0),
        _ => (1, 0),
    }
}

/// `ChestBlock.getConnectedBlockPos` for a chest whose `type` is not single:
/// where its other half belongs, whatever stands there now.
pub fn chest_connected_pos(
    state: &sunrise_world::chunk::BlockState,
    pos: [i32; 3],
) -> Option<[i32; 3]> {
    if !is_chest_block(&state.name) {
        return None;
    }
    let (dx, dz) = chest_half(state)?.connected_step()?;
    Some([pos[0] + dx, pos[1], pos[2] + dz])
}
