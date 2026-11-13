//! The `entity_entering_section` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `entity` moved from the 16x16x16 section `old_section` into `new_section` (NeoForge's
    /// `EntityEvent.EnteringSection`, vanilla's `EntitySectionStorage` callbacks on
    /// `Entity.setPosRaw`). Both are section coordinates (`SectionPos`: each block coordinate
    /// shifted right by 4), and `chunk_changed` is NeoForge's `didChunkChange`: the x or z
    /// section differs, so the entity is in another chunk column. `host` is the side that saw
    /// the move. Nothing can be cancelled: the entity is already in its new section.
    ///
    /// Told on each side once for every section change, from the
    /// section the entity was in before that move: two moves told apart are two events, and a
    /// move out and one back are two. The moves: an entity's own move in its tick (on the
    /// server every kind in the level's lists, the fishing hooks and the dragon; on a client
    /// each entity it shows, where its interpolation puts it), each move a server player sent,
    /// a teleport (`/tp`, a mod's `teleport_to`), a piston or shulker push, and on a client
    /// each packet that places an entity (its own player's teleport, a vehicle it drives). A
    /// mob's teleport in its own tick (an enderman's, a pet's to its owner, a shulker's) is a
    /// move of its own, before the walk that follows it, each one when it teleports twice; one
    /// collision-resolved walk is one move. An entity joining a level, changing level, or a
    /// player respawning starts in its section untold, as vanilla adds it to its section
    /// without this event; one that moves before its first tick is told. What a listener moves
    /// is told at the next move, not from inside its own call.
    ///
    /// Timing differs from vanilla for an entity's moves in its own tick (mobs, items and the
    /// other kinds that tick as a family): they are told at the end of its family's tick, the
    /// same crossings in the same order, later than the write, so a listener reads such an
    /// entity where the round left it, not at the destination of the crossing it is told.
    /// Teleports outside an entity's own tick, pushes, a player's moves and a client's packets are
    /// told at once.
    #[notes = r#"## Entering a section

`Hooks::entity_entering_section` (NeoForge's `EntityEvent.EnteringSection`) tells a mod each time an entity's
position puts it in another 16x16x16 section, on the side that saw it move. It is the moment vanilla moves the
entity between its section lists (`Entity.setPosRaw`, then the level callback's `onMove`). Use it to keep a mod's own
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

When it is told: at once for a teleport (`/tp`, `teleport_to`, `random_teleport` from outside the tick, an enderman's
or a shulker's teleport when hurt), a piston or shulker push, a server player's sent move and, on a client, a packet
that places an entity. A listener's own moves are told at the next update, never from inside its call.

Timing differs from vanilla for an entity's moves in its own tick (mobs, items and the other kinds that tick as a
family): they are told at the end of its family's tick, the same crossings in the same order, later than the write, so
a listener reads such an entity where the round left it, not at the destination of the crossing it is told."#]
    #[hook(
        side = "both",
        contract = "listen",
        neoforge = "EntityEvent.EnteringSection",
        cancellable = false
    )]
    fn entity_entering_section();
}
