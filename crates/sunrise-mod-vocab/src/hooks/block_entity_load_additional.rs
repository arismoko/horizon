//! Loading a mod block entity's saved payload on the server.

pub trait Hooks {
    /// The engine applied saved data to this mod block entity on the logical
    /// server (`BlockEntity.loadAdditional`). `tag` is the applied mod payload, and
    /// `event.subject` is a [`BlockEntityLevelReadScope`](mod_api::BlockEntityLevelReadScope):
    /// `with_view` lends a read-only [`BlockEntityReadView`](mod_api::BlockEntityReadView)
    /// carrying the instance's handle, `worldPosition`, level, block state and updated
    /// data, and `host()` borrows `&dyn LevelHost`, so a listener can store the applied
    /// data under its dimension and position, also after a later `/data merge`. The notice
    /// cannot change the tag or write to the world. On a holder without declared inventory slots,
    /// `tag` includes any saved `Items` field as mod-owned NBT, even rows that look
    /// like canonical item stacks. Lists keep their saved order. For a declared
    /// inventory, modeled stacks remain separate from the mod payload.
    ///
    /// This runs for chunk loads, block data commands, cloning, structure and
    /// block-item placement with data, and falling blocks that carry data.
    /// For a newly attached entity it precedes `block_entity_on_load`, which
    /// fires once per instance even for fresh placement without saved data.
    /// Direct Mod API storage edits (`merge_data`, `replace_data`) and
    /// transaction restores do not emit this notice. Client network updates
    /// use `block_entity_data_received` instead.
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        event_per_listener = true,
        scope = "block_entity",
        key = "id",
        target = "sunrise_gameplay_common::game::native_hooks::block_entity_load_additional::native_block_entity_load_additional",
        neoforge = "none (vanilla BlockEntity.loadAdditional; NeoForge adds no event for this moment)",
        version = 3
    )]
    fn block_entity_load_additional(
        id: mod_api::BlockEntityTypeKey,
        subject: mod_api::BlockEntityLevelReadScope<'_>,
        tag: &Compound,
    );
}
