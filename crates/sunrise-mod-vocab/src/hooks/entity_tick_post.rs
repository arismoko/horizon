//! The `entity_tick_post` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The entity `entity` of type `kind` ticked in the level `dimension` (NeoForge's
    /// `EntityTickEvent.Post`): one `entity_tick_pre` was told and did not cancel, including one
    /// that died or was removed in its tick. Every observed entity is bracketed individually right
    /// after its own tick. Told where `entity_tick_pre` is, on the server and on a client (`host`
    /// says which side). Version 4 fires whenever a mod subscribes to `entity_tick_post` directly
    /// (removing the `entity_tick_listened` opt-in poll hook, the `entity_tick_each` /
    /// `entity_tick_each_entity` registrar flags, and family batching) and carries one entity
    /// (`entity`, `kind`). Version 5 adds the server's frames, paintings, armor stands and thrown
    /// tridents.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::base::GameHooks::native_entity_tick_post",
        dispatch = "flat",
        side = "both",
        version = 5,
        contract = "listen",
        neoforge = "EntityTickEvent.Post",
        cancellable = false
    )]
    fn entity_tick_post(dimension: &str, entity: u32, kind: &str);
}
