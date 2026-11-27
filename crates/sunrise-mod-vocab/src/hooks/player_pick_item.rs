//! The `player_pick_item` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player picked an item from a block or entity, before it enters the hotbar. `pos` is
    /// zero for an entity; `entity_id` is zero for a block. The player's state is swapped in. Set
    /// `cancel` to leave the inventory and client unchanged.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_pick_item",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Paper's PlayerPickItemEvent)",
        cancellable = true
    )]
    fn player_pick_item(
        player: u128,
        name: &str,
        from_entity: bool,
        pos: [i32; 3],
        entity_id: i32,
        cancel: &mut bool,
    );
}
