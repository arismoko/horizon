//! The `block_entity_update_tag` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// What clients get of the block entity at `pos` when its `id` is
    /// yours (`BlockEntity.getUpdateTag`): set `tag` to it, read from
    /// `entity` alone. Asked on the server whenever a chunk holding it is
    /// sent and whenever its block changes, on the server thread; a client's copy of the block
    /// entity (made by `block_entity` there) merges it into its `nbt`.
    /// Leave it unset to send no tag of yours. A block change puts the one-shot fields of
    /// `BlockEntityView::set_client_update_fields` over it, and sends them alone when it is unset.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::base::native_block_entity_update_tag",
        receiver = "associated",
        dispatch = "flat",
        side = "server",
        version = 2,
        contract = "replace",
        out = "tag",
        scope = "block_entity",
        key = "entity",
        neoforge = "none (vanilla BlockEntity.getUpdateTag; NeoForge adds no event or extension for it); IBlockEntityExtension.handleUpdateTag is the client half"
    )]
    fn block_entity_update_tag(
        pos: [i32; 3],
        entity: BlockEntityDataView<'_>,
        tag: &mut Option<Compound>,
    );
}
