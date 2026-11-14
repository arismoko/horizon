//! The `entity_entering_section` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `entity` moved from the 16x16x16 section `old_section` into `new_section` (NeoForge's
    /// `EntityEvent.EnteringSection`, vanilla's `EntitySectionStorage` callbacks on
    /// `Entity.setPosRaw`). Both are section coordinates (`SectionPos`: each block coordinate
    /// shifted right by 4), and `chunk_changed` is NeoForge's `didChunkChange`: the x or z
    /// section differs, so the entity is in another chunk column. `host` is the side that saw
    /// the move. Nothing can be cancelled: the entity is already in its new section.
    ///
    /// An entity joining a level, changing level, or a player respawning starts in its section untold,
    /// as vanilla adds it to its section without this event; one that moves before its first tick is
    /// told.
    ///
    /// Each recorded crossing is reported in write order. During a mob's borrowed AI tick, section-
    /// crossing notifications are delivered in recorded write order after that individual mob tick
    /// returns and its owning list is restored, not inside the position setter; position reads
    /// temporarily show each recorded destination, while other state already reflects later work in
    /// that tick, so listeners must defer moving or removing entities and must not rely on write-time
    /// ordering against other tick side effects. Client interpolation and packet-movement
    /// notifications run at the implemented entity or packet-branch boundaries, not at every
    /// underlying position setter. An existing local player's packet teleport is reported after its
    /// position and velocity commit and before outbound teleport-acceptance and stop-mining packet
    /// callbacks. Lent item and projectile callbacks expose the committed position, not general
    /// resident-row access. Listeners must defer entity mutations; the engine does not automatically
    /// defer listener mutations.
    #[notes = r#"## Entering a section

`Hooks::entity_entering_section` (NeoForge's `EntityEvent.EnteringSection`) tells a mod each time an entity's
position puts it in another 16x16x16 section, on the side that saw it move. Vanilla moves the
entity between its section lists at `Entity.setPosRaw`, then the level callback's `onMove`; the borrowed-AI limitation
below means this hook does not always run at that moment. Use it to keep a mod's own
per-section or per-chunk index of entities without scanning every entity each tick:

```rust,ignore
fn entity_entering_section(&self, host: HostSide<'_>, event: &mut EntityEnteringSectionEvent) {
    if !event.chunk_changed {
        return;
    }
    let [x, _, z] = event.new_section;
    if let HostSide::Server(_) = host {
        self.index.lock().unwrap().insert(event.entity, (x, z));
    }
}
```

`old_section` and `new_section` are section coordinates (each block coordinate shifted right by 4), and
`chunk_changed` says the x or z section differs. Each position update is one check against the section the entity
was in before it, so a teleport and the walk after it are two events, and a move out of a section and back is two.
One walk the collisions shape is one update, told once at where it ended. A mob that teleports twice within its own
tick and then walks is told three crossings.

An entity joining a level, changing level or a player respawning starts in its section untold, as in vanilla; one
that moves before its first tick is told. With no listener the game files nothing, so the hook costs a
subscription check per position update.

During a mob's borrowed AI tick, section-crossing notifications are delivered in recorded write order after that individual mob tick returns and its owning list is restored, not inside the position setter; position reads temporarily show each recorded destination, while other state already reflects later work in that tick, so listeners must defer moving or removing entities and must not rely on write-time ordering against other tick side effects. Client interpolation and packet-movement notifications run at the implemented entity or packet-branch boundaries, not at every underlying position setter. An existing local player's packet teleport is reported after its position and velocity commit and before outbound teleport-acceptance and stop-mining packet callbacks. Lent item and projectile callbacks expose the committed position, not general resident-row access. Listeners must defer entity mutations; the engine does not automatically defer listener mutations."#]
    #[hook(
        side = "both",
        contract = "listen",
        neoforge = "EntityEvent.EnteringSection",
        cancellable = false
    )]
    fn entity_entering_section();
}
