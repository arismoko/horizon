//! `PistonHeadRenderer`: while a piston moves them, blocks are
//! `minecraft:moving_piston` block entities (drawn as nothing by the chunk
//! mesher); this draws each moved block at its interpolated offset.

impl crate::game::CommonGame<'_> {
    /// The section holding the block at `p` (`SectionPos.blockToSection`).
    pub fn section_of(&self, p: [i32; 3]) -> Option<(i32, i32, i32)> {
        let y = p[1].checked_sub(self.world.layout().min_y())?;
        Some((p[0] >> 4, y.div_euclid(16), p[2] >> 4))
    }
}
