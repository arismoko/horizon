//! The `can_take_item_through_face` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `WorldlyContainer.canTakeItemThroughFace`: whether the nonempty `stack` in `slot` of this
    /// mod block entity's container may leave through `face`. `Some(true)` or `Some(false)`
    /// replaces the declared answer, which is whether `ModContainer::outputs` lists the slot;
    /// `None` keeps it. A subscribed container refuses the take if the entity snapshot, block
    /// state or stack cannot be represented. The face's slots (`ModContainer::faces`,
    /// `getSlotsForFace`) still bound which slots are asked.
    ///
    /// This is a read-only query on the entity's position, block state, saved data and current
    /// slots, with a read-only `Host` for anything else the answer turns on (an attachment, the
    /// registries). The state is the block at this entity's own position. A hopper
    /// below pulling through `Down` (`HopperBlockEntity.canTakeItemFromContainer`) asks it, and
    /// so do `ITEMS` and `STORAGE` extracts through a side. An extract with no side does not:
    /// as NeoForge's unsided item handler, it skips the face rules. Menus use
    /// `menu_slot_rules` instead (`Slot.mayPickup`).
    #[hook(
        receiver = "read",
        host = "read",
        dispatch = "flat",
        side = "server",
        contract = "replace",
        out = "allowed",
        scope = "block_entity",
        key = "entity",
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_can_take_item_through_face",
        neoforge = "none (vanilla WorldlyContainer.canTakeItemThroughFace; the sided item capability wraps this decision)"
    )]
    fn can_take_item_through_face(
        entity: &ModBlockEntity,
        pos: BlockPos,
        state: mod_api::BlockState,
        slot: usize,
        stack: &mod_api::ItemStack,
        face: Direction,
        allowed: &mut Option<bool>,
    );
}
