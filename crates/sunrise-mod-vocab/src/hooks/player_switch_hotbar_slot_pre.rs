//! The `player_switch_hotbar_slot_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `PlayerSwitchHotbarSlotEvent.Pre`: a server player is switching from hotbar slot
    /// `old_slot` to `new_slot` (0 to 8), after the slot range check and before the
    /// selected slot changes. Fired only when the slot differs. The player's state is not
    /// swapped in. Set `cancel` to keep the old slot; the client is sent that slot again,
    /// and `player_switch_hotbar_slot_post` doesn't fire.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_switch_hotbar_slot_pre",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "PlayerSwitchHotbarSlotEvent.Pre",
        paper = "PlayerItemHeldEvent",
        cancellable = true
    )]
    fn player_switch_hotbar_slot_pre(
        player: u128,
        player_entity_id: i32,
        old_slot: i32,
        new_slot: i32,
        cancel: &mut bool,
    );
}
