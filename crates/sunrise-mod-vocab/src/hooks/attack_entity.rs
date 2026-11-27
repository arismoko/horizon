//! The `attack_entity` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player attacks the entity `entity` (`Entity.getId`) of type `kind`
    /// (`minecraft:item_frame`, `minecraft:cow`, `minecraft:player`) standing
    /// at `pos`: `ServerGamePacketListenerImpl.handleInteract`'s attack, once
    /// in reach, where `Player.attack` starts. The swing has not begun, so the
    /// attacking player's attack strength is still the charge the blow uses
    /// (vanilla reads it at the start of `Player.attack`; the swing resets it
    /// afterwards). Set `canceled` and the blow does nothing (no damage, no
    /// knockback, the frame keeps its item). Server-side, for every player of
    /// the world (`host.players().acting_player()` says who).
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_attack_entity",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "AttackEntityEvent"
    )]
    fn attack_entity(entity: i32, kind: &str, pos: [f64; 3], canceled: &mut bool);
}
