//! The `player_shear_entity` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player shearing a mob is about to do so. Set `cancel` to leave the mob and shears unchanged.
    /// The guest's state is swapped in. The listener may change the world: straight
    /// after it returns without cancelling, the game finds the mob again by its
    /// entity id and checks that it still exists, is not removed, and that the held
    /// stack keeps its item and count; if not, the click is spent and nothing is
    /// sheared.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_shear_entity",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerShearEntityEvent)",
        cancellable = true
    )]
    fn player_shear_entity(
        player: u128,
        name: &str,
        entity_id: i32,
        off_hand: bool,
        cancel: &mut bool,
    );
}
