//! The `creative_slot` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A creative-mode player sets a slot of their inventory from the
    /// creative screen (`ServerGamePacketListenerImpl.handleSetCreativeModeSlot`):
    /// `slot` is the inventory menu's slot (5 to 8 armour, 9 to 35 the
    /// inventory, 36 to 44 the hotbar, 45 the off hand; below 0 is a stack
    /// thrown out of the screen, which is dropped) and `item` (`minecraft:air`
    /// for an emptied slot) and `count` what is put there. Heard once the
    /// packet passed vanilla's checks (a creative player, a real slot, a valid
    /// stack) and before the slot changes or the stack drops. Set `cancel`
    /// and the packet is ignored. The player's state is swapped in for the
    /// call, so `host.players().acting_player()`, `PlayerRef::tell` and the item
    /// helpers reach them. Not heard: stacks a
    /// creative player gets other ways (middle-click pick block, `/give`).
    /// Server-side, for every player of the world.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_creative_slot",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        neoforge = "none (vanilla ServerGamePacketListenerImpl.handleSetCreativeModeSlot; NeoForge adds no event or extension for it)"
    )]
    fn creative_slot(
        player: u128,
        name: &str,
        slot: i32,
        item: &str,
        count: i32,
        cancel: &mut bool,
    );
}
