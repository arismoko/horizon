//! The `mod_container_viewers` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The number of players with the mod container at `pos` of `dimension`
    /// open went from `previous` to `viewers` (vanilla's
    /// `ContainerOpenersCounter`, as a barrel keeps it): 0 to more than 0 is
    /// `onOpen`, back to 0 is `onClose`, any change `openerCountChanged`, so a
    /// block can show itself open while `viewers` is not 0. Incremented
    /// synchronously when a non-spectator opens the menu, decremented when
    /// it closes (including disconnect), and rechecked every five ticks while
    /// open, including copper golems in range. Rechecks also report unchanged
    /// counts. Zero crossings post container_open/container_close game events:
    /// the opening or closing player is the source, none on a recheck.
    /// Server-side; open/close runs with the player's state in place.
    #[notes = "Version 2 follows ContainerOpenersCounter: synchronous open/close, five-tick rechecks, and game events on zero crossings instead of a per-tick recount."]
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_mod_container_viewers",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        version = 2,
        neoforge = "none (vanilla ContainerOpenersCounter; NeoForge adds no event or extension for it); PlayerContainerEvent.Open and Close fire per player"
    )]
    fn mod_container_viewers(dimension: &str, pos: [i32; 3], viewers: u32, previous: u32);
}
