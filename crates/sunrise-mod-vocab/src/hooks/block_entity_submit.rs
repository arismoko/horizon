//! The `block_entity_submit` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Draw the mod block entity at `pos`, of block `block`
    /// (`mymod:machine`), on this client: once a frame while it is in view
    /// (vanilla's `BlockEntityRenderer.submit`: within its view distance, 64 blocks unless
    /// `block_entity_view_distance` says otherwise, its chunk
    /// faded in, and its bounds, `block_entity_render_bounding_box`,
    /// in the frustum), when its `id` is yours. `state` is the block's state
    /// (`blockEntity.getBlockState()`: read its `facing` with
    /// `native_block_state_property`), `entity` holds what the server sent
    /// (`block_entity_update_tag`) without copying it. Add geometry with `draw.quad`, `draw.cube`,
    /// `draw.block` (taking a resolved `BlockState`) and `draw.item` under
    /// `draw`'s pose stack, which starts at the
    /// block's lower north-west corner, in blocks; `draw` also has the partial
    /// tick, the game time and the block's light.
    /// When `DrawContext::draw_scene` reuses this renderer, `entity` is supplied
    /// by the scene, the draw is full bright, and `pos` is scene-local, not a
    /// position to look up in the client level. `BlockEntityDrawContext::block_entity` reuses it
    /// the same way for a block entity a mod carries: `entity` is the `SceneBlockEntity`, `pos`
    /// is the draw's own block, and `draw` is one level deep (a nested `block_entity` is refused).
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_block_entity_submit",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 3,
        contract = "listen",
        scope = "block_entity",
        key = "block",
        neoforge = "EntityRenderersEvent.RegisterRenderers"
    )]
    fn block_entity_submit(
        pos: [i32; 3],
        block: &str,
        state: BlockState,
        entity: BlockEntityDataView<'_>,
        draw: &mut dyn BlockEntityDrawContext,
    );
}
