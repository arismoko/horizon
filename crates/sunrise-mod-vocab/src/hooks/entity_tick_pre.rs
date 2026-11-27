//! The `entity_tick_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The entity `entity` of type `kind` (`minecraft:zombie`, `minecraft:item`) is about to tick
    /// in the level `dimension` (NeoForge's `EntityTickEvent.Pre`, posted right before each
    /// entity's tick in `tickNonPassenger`).
    /// Told on the server for the mobs, players, item entities, minecarts, boats, item and glow
    /// item frames, paintings, armor stands, thrown tridents, general entity
    /// rows and projectiles that tick in an entity-ticking chunk, and the entities the server
    /// keeps without simulating them (displays, markers, interactions), whose tick is only told
    /// (not a player's own boat or cart, which ticks with the player); and on a client for the
    /// mobs, other players, projectiles, tridents and displays it ticks; `host` says which side.
    /// Every observed entity is bracketed individually (`Pre(A)`, tick `A`, `Post(A)`, `Pre(B)`,
    /// ...), matching NeoForge's `EntityTickEvent.Pre` / `.Post` timing. On the server, player
    /// Pre runs before the observed parts of `ServerPlayer.tick`, not the connection body's
    /// `player_tick_pre`. The local client player is not yet observed here.
    /// Return [`Flow::Cancel`] to skip this entity's observed tick work and suppress its Post.
    /// Removing a general entity during its own Pre skips its tick and Post and dismounts its
    /// passengers unless dismount is vetoed. An eligible former passenger can still tick as a
    /// root in this pass if it has not ticked and its join-order slot is still ahead. An
    /// already-passed slot is not revisited.
    /// Version 4 fires whenever a mod subscribes to `entity_tick_pre` directly (removing the
    /// `entity_tick_listened` opt-in poll hook, the `entity_tick_each` / `entity_tick_each_entity`
    /// registrar flags, and family batching) and carries one entity (`entity`, `kind`) returning
    /// [`Flow`]. Version 5 adds the server's item and glow item frames, paintings, armor stands
    /// and thrown tridents.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::base::GameHooks::native_entity_tick_pre",
        dispatch = "flat",
        side = "both",
        version = 5,
        contract = "cancel",
        out = "canceled",
        neoforge = "EntityTickEvent.Pre",
        cancellable = true
    )]
    fn entity_tick_pre(dimension: &str, entity: u32, kind: &str, canceled: &mut bool);
}
