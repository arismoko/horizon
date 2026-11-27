use sunrise_level::containers::{BlockEntity, Crafter};

impl crate::game::CommonGame<'_> {
    pub fn crafter_open(&self) -> bool {
        matches!(self.container_kind(), Some(BlockEntity::Crafter(_)))
    }
    pub fn crafter_at(&self, pos: (i32, i32, i32)) -> Option<&Crafter> {
        match self.world.block_entities.get(&pos)? {
            BlockEntity::Crafter(c) => Some(c),
            _ => None,
        }
    }
}
