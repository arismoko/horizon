//! The `can_place_item` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `Container.canPlaceItem`: whether `slot` of this mod block entity's container admits
    /// the offered nonempty `stack`. `Some(true)` or `Some(false)` replaces the declared
    /// `ModContainer::insert_rules` answer; `None` keeps it. A subscribed slot refuses
    /// insertion if the entity snapshot, block state or offered stack cannot be represented.
    /// Face, input-slot and stack limits still apply.
    ///
    /// This is a read-only query on the entity's position, block state, saved data and
    /// current slots, with no Host. The state is the block at this entity's own position,
    /// not the block of the hopper, dropper or other caller.
    /// Hoppers, droppers, default `ITEMS` and `STORAGE` inserts, and nonempty `/loot insert`
    /// and `/loot replace block` offers ask it. Inserts query with the remaining offer
    /// before clamping to room. Default `STORAGE.space_for` does not ask this hook: for a
    /// subscribed slot it bounds insertion by physical room after face and input checks.
    /// Menus use `menu_slot_rules` instead (`Slot.mayPlace`).
    ///
    /// Seed a filter from an accepted write, not this query. Default `ITEMS` and `STORAGE`
    /// writes to engine-managed slots emit `contents_changed` on commit (after source
    /// settlement when the commit happens during `menu_quick_move`). A provider writing its
    /// own storage can issue an explicit `set_changed` or seed the filter itself.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::can_place_item::native_can_place_item",
        receiver = "associated",
        dispatch = "flat",
        side = "server",
        version = 2,
        contract = "replace",
        out = "allowed",
        scope = "block_entity",
        key = "entity",
        neoforge = "none (vanilla Container.canPlaceItem; the item capability wraps this decision)"
    )]
    fn can_place_item(
        entity: &ModBlockEntity,
        pos: BlockPos,
        state: mod_api::BlockState,
        slot: usize,
        stack: &mod_api::ItemStack,
        allowed: &mut Option<bool>,
    );
}
