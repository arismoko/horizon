//! What a crash save decided about the players a panic interrupted.

#[derive(Default)]
pub struct CrashSave {
    // An interrupted player is still swapped in as the acting player: the game's own stats and advancements are not written.
    acting: bool,
    // level.dat is written without a new player file: the owner was interrupted, or the hub is lost.
    pub keep_owner_file: bool,
    hub_lost: bool,
    // Interrupted players' connections: their player-data, stats and advancement saves are skipped.
    excluded: std::collections::BTreeSet<u32>,
}

impl CrashSave {
    /// The game's own stats and advancements belong to an interrupted player.
    pub fn holds_interrupted(&self, acting: Option<u32>) -> bool {
        self.acting && acting.is_some()
    }

    /// Whether the player on `conn` is interrupted: their files are not written.
    pub fn excludes(&self, conn: u32) -> bool {
        self.excluded.contains(&conn)
    }

    /// Records that an interrupted player is swapped in as the acting player.
    pub fn note_acting(&mut self, acting: bool) {
        self.acting |= acting;
    }

    /// Marks the hub lost and the owner file kept; false when it was already lost.
    pub fn lose_hub(&mut self) -> bool {
        if self.hub_lost {
            return false;
        }
        self.hub_lost = true;
        self.keep_owner_file = true;
        true
    }

    /// Excludes `conn`; false when it was already excluded.
    pub fn exclude(&mut self, conn: u32) -> bool {
        self.excluded.insert(conn)
    }
}
