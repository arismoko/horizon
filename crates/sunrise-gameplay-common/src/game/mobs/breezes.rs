//! Breeze particles (`Breeze.tick`): block dust at its feet while it stands,
//! a burst of twenty while it slides and a short trail along a long jump.
use super::BlockStateId;
use sunrise_sim::mobs::Mob;

impl crate::game::CommonGame<'_> {
    /// `Breeze.emitGroundParticles`'s block: `getInBlockState`, else
    /// `getBlockStateOn`, which is `Entity.getOnPos(1.0E-5F)` — the supporting
    /// block the feet rest on, or the cell just below them. A fence, a wall and
    /// a fence gate have a 1.5-block collision, so a breeze on one has its feet
    /// half a block above that block's own cell, where the cell below them is
    /// air and only the supporting block has the material.
    pub fn breeze_ground(&self, m: &Mob) -> ([i32; 3], BlockStateId) {
        let feet = [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ];
        let in_block = self.world.block(feet[0], feet[1], feet[2]);
        if !self.res.facts().is_air(in_block) {
            return (feet, in_block);
        }
        let at = m.ground.supporting_block.map_or(
            [
                feet[0],
                (m.position.y - f64::from(1.0e-5_f32)).floor() as i32,
                feet[2],
            ],
            |p| [p.x, p.y, p.z],
        );
        (at, self.world.block(at[0], at[1], at[2]))
    }
}
