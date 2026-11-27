//! The `random_tick` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An authoritative server's `ServerLevel.tickChunk` is about to
    /// random-tick the block `state` at `pos` (`BlockState.randomTick`):
    /// only blocks that tick randomly, a mod's as its vanilla template block
    /// does. Bone meal's random ticks (`StemBlock.performBonemeal`) don't
    /// fire it. Change the block with `LevelWriteHost::set_block_property` and set
    /// `handled` to skip the block's own random tick (a mod crop growing its
    /// own way); a block changed without `handled` takes its own random tick
    /// as it is now, if it still ticks randomly. Lava's fluid random tick
    /// runs either way.
    /// Its disabled owner's subscribed Part keeps the block inert instead of running the template tick.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "handled",
        scope = "block",
        inert = "*handled = true;",
        key = "state",
        target = "sunrise_gameplay_common::game::native_hooks::base::GameLevel<'_>::native_random_tick",
        neoforge = "none (vanilla BlockBehaviour.randomTick; NeoForge adds no event or extension for it)"
    )]
    fn random_tick(
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockState,
        handled: &mut bool,
        #[engine] ticks: &mut Ticks,
    );
}
