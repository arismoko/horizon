//! Serialization of a mod block entity's owned payload.

pub trait Hooks {
    /// Write live mod state into `tag`, a copy of the instance's stored payload
    /// (`BlockEntity.saveAdditional`). Edits affect only the output; the entity,
    /// its inventory, identity, position, and engine-managed attachments and
    /// shapes are not changed. Parts of the entity's type run in dispatch order.
    /// For a holder without declared inventory slots, `tag` holds any persisted
    /// `Items` as mod-owned NBT. Canonical-looking item rows keep their saved list
    /// order; they are not reconstructed in slot order. The listener may replace or
    /// remove this field. Declared slots remain engine-owned inventory, serialized
    /// by the engine after the listeners run.
    /// `event.pos` and `event.level` are the instance's own `worldPosition` and level, read-only and
    /// filled by the engine, so a listener can find its live state by dimension and position, including
    /// during unload or a removed block's loot roll. Every call has a level: the hook is not asked
    /// without one.
    /// Malformed payloads, including overlong modified-UTF-8 names and
    /// strings, fall back to the stored payload before any output is used.
    /// If the region writer cannot encode or
    /// admit the complete chunk record, it retries that snapshot with stored
    /// block-entity payloads instead of listener edits.
    ///
    /// Asked on the logical server thread for chunk saves (including unload),
    /// command reads, and copies into items, loot or templates. The read-only
    /// level host cannot write or trigger another save. Use `tag` for stored
    /// fields during unload or a removed block's loot roll, when the level
    /// may no longer expose the entity.
    /// This is not the one-time attachment notice `block_entity_on_load`, nor
    /// the client sync payload `block_entity_update_tag`.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        event_per_listener = true,
        scope = "block_entity",
        key = "id",
        target = "sunrise_gameplay_common::game::native_hooks::block_entity_save_additional::GameLevel<'_>::native_block_entity_save_additional",
        neoforge = "none (vanilla BlockEntity.saveAdditional; NeoForge adds no event for this moment)",
        version = 4
    )]
    fn block_entity_save_additional(
        id: mod_api::BlockEntityTypeKey,
        entity: BlockEntityRef,
        pos: [i32; 3],
        level: LevelRef,
        tag: &mut Compound,
    );
}
