//! The `entity_leave_level` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An entity leaves the server level `dimension` (NeoForge's
    /// `EntityLeaveLevelEvent`): vanilla's `onTrackingEnd`, when it is removed
    /// for any reason (picked up, merged, despawned, burnt, blown up, killed,
    /// taken by a hopper or a mob, gone through a portal) or its chunk unloads,
    /// a level closing with nobody in it included. Not for the entities still
    /// loaded when the server stops, as in vanilla. `entity` is its ID and
    /// `kind` its type (`minecraft:item`); it is no longer in the level's lists.
    /// It only hears. It tells every kind `entity_join_level` does
    /// (`kind` is the type), each once, when the game next notices it gone
    /// (the tick or action that removed
    /// it). Inside a transaction (`Host::transaction`) opened in the entity's
    /// own level an entity's removal takes effect, and this hook fires, at that
    /// transaction's root commit, not at the removal call (an entity in another
    /// level, as by a command run `in` it, leaves at once): from the call on its handle and selectors no longer find
    /// it. An aborted transaction removes nothing and tells nothing. An entity
    /// the transaction spawned and removed is never told to join, so it never
    /// leaves. Check `kind`. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_entity_leave_level",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "EntityLeaveLevelEvent",
        version = 3
    )]
    fn entity_leave_level(dimension: &str, entity: u32, kind: &str);
}
