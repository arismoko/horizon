//! The `crop_grow` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A crop-like block at `pos` is about to grow on a random tick
    /// (NeoForge's `CropGrowEvent.Pre`): crops, stems, nether wart, cocoa,
    /// sweet berries and bamboo at their own position; cactus and chorus
    /// flowers at the block above; sugar cane at its own position; kelp,
    /// weeping and twisting vines and cave vines at the block they grow
    /// into. `state` is the growing block. Leave `result` at `Default` for
    /// vanilla's own chance, or set `True` to grow now or `False` to stop
    /// it (`DO_NOT_GROW`). Vanilla's random roll is drawn either way, in
    /// vanilla's order. Server-side.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "CropGrowEvent.Pre",
        target = "sunrise_gameplay_common::game::native_hooks::crop_grow::GameLevel<'_>::native_crop_grow_pre"
    )]
    fn crop_grow_pre(
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockState,
        result: &mut TriState,
    );
    /// A crop-like block grew (NeoForge's `CropGrowEvent.Post`).
    /// Diverges from NeoForge: original_state is always the pre-growth state.
    /// `state` is what is at `pos` now. Sugar cane reports the new cane above
    /// it, and kelp and vines the new head, while `original_state` names the
    /// growing plant before growth. A full-age stem fires even without fruit;
    /// sugar cane fires only when adding a cane, not when only aging. Cactus
    /// and chorus report the plant's position. Server-side.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "CropGrowEvent.Post",
        version = 2,
        target = "sunrise_gameplay_common::game::native_hooks::crop_grow::GameLevel<'_>::native_crop_grow_post"
    )]
    fn crop_grow_post(
        dimension: LevelDimension,
        pos: BlockPos,
        original_state: BlockState,
        state: BlockState,
    );
}
