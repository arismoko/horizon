//! The `player_change_beacon_effect` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player changed a beacon's selected effects, after they were stored. Each selected
    /// effect is an `Option<EffectKey>`; None means no effect. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_change_beacon_effect",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_change_beacon_effect(
        player: u128,
        name: &str,
        pos: [i32; 3],
        primary: Option<EffectKey>,
        secondary: Option<EffectKey>,
    );
}
