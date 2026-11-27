use sunrise_world::registry::BlockStateId;

/// Whether a block type provides the composter's state-backed container ports.
pub fn is_composter(kind: &str) -> bool {
    kind == "minecraft:composter"
}

/// State-backed container feedback, published only by the outermost successful transaction.
pub enum Effects {
    Insert {
        pos: [i32; 3],
        before: BlockStateId,
        after: BlockStateId,
    },
    Empty {
        pos: [i32; 3],
        after: BlockStateId,
    },
}
