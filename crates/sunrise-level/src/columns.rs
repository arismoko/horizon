//! A terrain column's surface, as generators and saved chunks report it.
use sunrise_world::registry::BlockStateId;

/// Top solid face and optional fluid face, in absolute block coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    pub height: i32,
    pub state: BlockStateId,
    pub water: Option<(i32, BlockStateId)>,
    pub biome: u16,
}
