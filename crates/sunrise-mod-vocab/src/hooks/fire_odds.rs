//! The `fire_odds` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// How fire treats the block `state` at `pos` on either side
    /// (`FireBlock.setFlammable(block, igniteOdds, burnOdds)`). The client
    /// reads ignite odds for placement and shape prediction; only the server burns blocks.
    /// From the block's side `face`, the one facing the fire or the air fire may take, put
    /// `(ignite, burn)` in `odds` for your own blocks, and vanilla's table
    /// isn't read. Ignite odds above zero let fire stand beside the block and
    /// spread into the air around it (planks 5, wool 30, leaves 30); burn odds
    /// are how fast fire destroys it (planks 20, wool 60, leaves 60). `(0, 0)`
    /// never burns, and a face with ignite odds of zero isn't flammable. Negative
    /// answers become zero; burn odds cap at 300 (horizontal rolls are 300,
    /// vertical rolls 250), and spread speed uses saturating arithmetic. A waterlogged state never burns and
    /// isn't asked. Answer only for your own blocks and leave `odds` empty
    /// otherwise; the level is read-only. Asked hundreds of times per fire
    /// tick: keep it short.
    #[hook(receiver = "read", dispatch = "flat", side = "both",
        version = 2,
        contract = "replace",
        out = "odds",
        scope = "block",
        key = "state",
        host = "level",
        target = "sunrise_gameplay_common::game::native_hooks::fire_odds::FireOddsLevel<'_>::native_fire_odds",
        neoforge = [
            "IBlockExtension.getFlammability",
            "IBlockExtension.getFireSpreadSpeed",
            "IBlockExtension.isFlammable"
        ]
    )]
    fn fire_odds(
        dimension: LevelDimension,
        state: BlockState,
        pos: BlockPos,
        face: Direction,
        odds: &mut Option<(i32, i32)>,
    );
}
