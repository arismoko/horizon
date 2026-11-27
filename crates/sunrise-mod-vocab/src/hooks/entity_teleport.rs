//! The `entity_teleport` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server entity was moved by a same-level teleport command, after its
    /// position changed. `entity` is its network id; `from` and `to` are
    /// coordinates in `dimension`. This hook reports command teleports that
    /// use Sunrise's same-level target path; it does not change vanilla's
    /// movement.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_entity_teleport",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_teleport(dimension: &str, entity: u32, from: [f64; 3], to: [f64; 3]);
}
