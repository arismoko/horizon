//! Breaking blocks: a held tool mining a block, `break_block` and the player's break with its drops
//! (`Block.popResource` for a broken block), the destroy-progress cracks shown to everyone, and a
//! bundle dropping its content.

impl crate::game::ClientGame<'_> {
    /// `ClientLevel.destroyBlockProgress`: any stage outside 0..10 clears the breaker.
    pub fn destroy_block_progress(&mut self, breaker: i32, pos: [i32; 3], progress: i32) {
        if (0..10).contains(&progress) {
            let stage = progress as u8;
            if let Some(crack) = self.client.destroying.get_mut(&breaker)
                && crack.pos == pos
                && crack.stage == stage
            {
                crack.updated_at = self.common.world.game_time;
                return;
            }
            // Publish the stage before dispatch: a same-stage client setter
            // from this block's part must refresh it, not enter the hook again.
            self.client.destroying.insert(
                breaker,
                crate::game::client_cracks::ClientCrack {
                    pos,
                    stage,
                    updated_at: self.common.world.game_time,
                    positions: Vec::new(),
                },
            );
            self.with_break_progress_positions(pos, stage, |game, positions| {
                if let Some(crack) = game.client.destroying.get_mut(&breaker)
                    && crack.pos == pos
                    && crack.stage == stage
                {
                    crack.positions = positions;
                }
            });
        } else {
            self.client.destroying.remove(&breaker);
        }
    }
}
