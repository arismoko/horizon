//! An end crystal's construction and the blocks its tick reads.

impl crate::game::ServerGame<'_> {
    /// `EndCrystal.tick`'s `applyEffectsFromBlocks` for crystal `i`, which never
    /// moves: a cobweb or powder snow in its box (2 x 2 x 2, deflated by
    /// `1.0E-5F`) calls `makeStuckInBlock`, which resets its fall distance.
    /// Powder snow's inside shape for a fall over 2.5 is only 0.9 tall
    /// (`FALLING_COLLISION_SHAPE`). A bubble column's reset and `stepOn` for a
    /// grounded crystal are not ported.
    pub(in crate::game::server_side) fn crystal_block_effects(&mut self, i: usize) {
        let (p, fall) = (
            self.common.crystals[i].position,
            self.common.crystals[i].fall.1,
        );
        let e = f64::from(1.0e-5_f32);
        let lo = [p.x - 1.0 + e, p.y + e, p.z - 1.0 + e].map(|v| v.floor() as i32);
        let hi = [p.x + 1.0 - e, p.y + 2.0 - e, p.z + 1.0 - e].map(|v| v.floor() as i32);
        let facts = self.common.res.facts();
        let stuck = (lo[0]..=hi[0]).any(|x| {
            (lo[1]..=hi[1]).any(|y| {
                (lo[2]..=hi[2]).any(
                    |z| match &*facts.get(self.common.world.block(x, y, z)).name {
                        "minecraft:cobweb" => true,
                        "minecraft:powder_snow" => fall <= 2.5 || p.y + e < f64::from(y) + 0.9,
                        _ => false,
                    },
                )
            })
        });
        if stuck {
            self.common.crystals[i].fall.1 = 0.0;
        }
    }
}
