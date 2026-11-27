//! The `smoke_source` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `CampfireBlock.isSmokeSource` on both sides for the
    /// block `state` under a campfire, as the campfire is placed or the block
    /// under it changes: `Some(true)` for your own blocks makes the campfire a
    /// signal fire whose smoke rises higher, as over a hay bale. The level is
    /// read-only. The client asks during placement prediction. `None` keeps
    /// vanilla's hay-bale check; `Some(false)` overrides it.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "both",
        version = 2,
        contract = "replace",
        out = "smoke",
        scope = "block",
        key = "state",
        target = "sunrise_gameplay_common::game::native_hooks::base::GameLevel<'_>::native_smoke_source",
        neoforge = "none (vanilla CampfireBlock.isSmokeSource; NeoForge adds no event or extension for it)"
    )]
    fn smoke_source(dimension: LevelDimension, state: BlockState, smoke: &mut Option<bool>);
}
