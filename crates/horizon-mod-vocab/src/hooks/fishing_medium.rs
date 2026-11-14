//! The `fishing_medium` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A fishing bobber `bobber` at `position` in `level` is flying or floating outside water
    /// (`FishingHook.tick`'s fluid test found none), and a mod may say it floats and fishes in a
    /// medium there: lava, the void under an island, a cloud. `owner` is the player who cast it
    /// and `owner_position` where they stand (their feet); a client may know neither for another
    /// player's bobber. Answer a [`FishingMedium`] to make the spot a medium, or None to leave
    /// vanilla's (the bobber falls). The first mod to answer, in dependency order, wins.
    ///
    /// In the block of `surface` or below the bobber floats, waits, bites and is reeled in as in
    /// still water; a surface that is not finite is no medium. On the server a catch rolls
    /// `loot_table` (vanilla's `minecraft:gameplay/fishing` for None, and for an id that is
    /// malformed or names no table, with one warning naming the mod and the id) and the wait for
    /// a fish draws from `lure_ticks`. A client runs the same test to draw the bobber floating
    /// and uses only `surface`. A bobber answered None later, out of water, rolls vanilla's table
    /// again. Never asked for a bobber in water or hooked to an entity.
    ///
    /// Asked every tick for each such bobber, on the server with the caster's state swapped in
    /// (`owner` is them) and on each client that draws it, so answer from the level and the two
    /// positions, the same on both sides. With no listener nothing is converted or allocated.
    /// Diverges from upstream: no NeoForge counterpart (its fishing events come after the roll).
    #[hook(
        side = "both",
        contract = "replace",
        out = "medium",
        answered_by = "mod",
        neoforge = "none (NeoForge's ItemFishedEvent fires after the loot roll, in FishingHook.retrieve)"
    )]
    fn fishing_medium();
}
