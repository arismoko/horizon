//! The `player_item_frame_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player changed an item frame after its contents or rotation changed. `action` is
    /// `put`, `rotate`, `remove_item` or `break`; `item` is empty when no item remains, and `rotation`
    /// is the resulting step. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_item_frame_change",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_item_frame_change(
        player: u128,
        name: &str,
        entity_id: u32,
        action: &str,
        item: &str,
        rotation: u8,
    );
}
