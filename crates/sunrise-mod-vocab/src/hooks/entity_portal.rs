//! The `entity_portal` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server entity finished its portal wait and is beginning to leave
    /// `dimension` through `pos`. The destination follows the portal rules.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_entity_portal",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_portal(dimension: &str, entity: u32, pos: [i32; 3]);
}
