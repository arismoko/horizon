use super::*;

use super::mod_api_host::FireOddsLevel;

impl FireOddsLevel<'_> {
    /// How fire treats the block `state` at `pos` on either side
    /// (`FireBlock.setFlammable(block, igniteOdds, burnOdds)`). The client
    /// reads ignite odds for placement and shape prediction; only the server burns blocks.
    /// From the block's side `face`, the one facing the fire or the air fire may take, put
    /// `(ignite, burn)` in `odds` for your own blocks, and vanilla's table
    /// isn't read. Ignite odds above zero let fire stand beside the block and
    /// spread into the air around it (planks 5, wool 30, leaves 30); burn odds
    /// are how fast fire destroys it (planks 20, wool 60, leaves 60). `(0, 0)`
    /// never burns, and a face with ignite odds of zero isn't flammable. Negative
    /// answers become zero, burn odds cap at 300, and ignite odds have no cap
    /// (spread arithmetic saturates). A waterlogged state never burns and isn't asked. Answer only for your own blocks and leave `odds` empty
    /// otherwise; the level is read-only. Asked hundreds of times per fire
    /// tick: keep it short.
    #[inline(always)]
    pub(in crate::game) fn native_fire_odds(
        &self,
        dimension: LevelDimension,
        state: BlockStateId,
        pos: BlockPos,
        face: Direction,
        odds: &mut Option<(i32, i32)>,
    ) {
        self.fire_fire_odds(dimension, state, pos, face, odds);
    }
}
