//! `BlockEntity.preRemoveSideEffects`: the declaration the mod API generator reads.

pub trait Hooks {
    /// On the authoritative server, the block at `pos` is being replaced.
    /// `state` is its outgoing state. Read the retiring block entity through
    /// `entity`; its attachments are still available through the level host.
    /// Use `effects` to suppress the base container scatter, scatter additional
    /// slots, spawn positioned item entities or award experience. An invalid
    /// contribution does not change any other contribution.
    ///
    /// The game scatters base contents first unless suppressed, then applies
    /// contributed slots, positioned items and experience in that order.
    /// Calls within each group keep their order. An empty slot takes the
    /// position draws of `Containers.dropItemStack` without spawning anything.
    /// Nothing from a panicking Part is applied. The level host permits reads
    /// and random draws, not arbitrary writes. Update flag 256 skips the call.
    #[notes = r#"- `pre_remove_side_effects` is the server's `BlockEntity.preRemoveSideEffects`: the block is being
  replaced or broken with side effects (not under flag 256). Read the outgoing row via `event.entity`
  and its attachments with `key.get_in(host, Holder::BlockEntity(pos), read)`. Submit effects with `event.effects`.
  Base contents scatter first unless suppressed; contributed slots, positioned items with default pickup delay,
  and experience follow in that order. Order within each group is preserved. `drop_slot(None)` draws an empty
  slot's scatter position without spawning. An invalid stack or nonfinite position refuses only that call.
  A Part that contributes nothing does not cause removal effects; contributions from a panicking Part are discarded.
  A ticking or menu-hook block entity can be removed through the Host; don't drop its contents separately in that callback."#]
    #[hook(
        side = "server",
        dispatch = "flat",
        contract = "listen",
        staged_surface = "effects",
        version = 6,
        scope = "block_entity",
        key = "entity",
        target = "sunrise_gameplay_common::game::native_hooks::pre_remove_side_effects::GameLevel<'_>::native_pre_remove_side_effects",
        neoforge = "none (vanilla BlockEntity.preRemoveSideEffects; NeoForge adds no event or extension for it)"
    )]
    fn pre_remove_side_effects(
        pos: BlockPos,
        state: BlockState,
        entity: BlockEntityDataView<'_>,
        effects: &mut dyn BlockRemovalEffects,
    );
}
