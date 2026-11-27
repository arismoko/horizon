//! Refusals when a mod writes a player's inventory, ender chest or saved data.

/// Why giving a stack or writing a player's inventory or ender chest was refused.
/// Nothing was written when one comes back.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum PlayerDataRefusal {
    /// The handle is stale or the server player's record is unavailable.
    Stale,
    /// The call was made on a client; these writes are the server's.
    Client,
    /// Giving a stack requires a living player. Direct slot setters don't use this refusal.
    Dead,
    /// The slot is past the last one of that container.
    SlotOutOfRange {
        /// The slot asked for.
        slot: u32,
        /// The last slot there is: 42 for the inventory, 26 for the ender chest.
        last: u32,
    },
    /// The game's item conversion boundary refused the supplied stack: an item the world
    /// lacks, a count or damage the item does not allow, malformed components.
    Stack(crate::StackWriteError),
}

impl From<crate::StackWriteError> for PlayerDataRefusal {
    fn from(error: crate::StackWriteError) -> Self {
        Self::Stack(error)
    }
}

impl crate::NoBackend for PlayerDataRefusal {
    fn no_backend() -> Self {
        Self::Stale
    }

    fn wrong_side() -> Self {
        Self::Client
    }
}

/// Why [`crate::PlayerRef::set_persistent_data_key`] wrote nothing.
///
/// Living and dead online players on the server both accept persistent data writes,
/// matching NeoForge's `Entity.getPersistentData()` and `ServerPlayer.restoreFrom()`.
// no wire: reports an in-process edit to the active player's saved data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlayerPersistentDataError {
    /// The player is no longer available to this host, or no world is in scope.
    Stale,
    /// The call was made on a client; this edit is the server's.
    Client,
    /// The value would nest a list or compound at depth 512 or more in the player file.
    TooDeep,
    /// The entry contains an NBT `End` value, a string or key exceeding the file's modified
    /// UTF-8 field width, or a collection whose length does not fit the file's count field.
    InvalidNbt,
}

impl crate::NoBackend for PlayerPersistentDataError {
    fn no_backend() -> Self {
        Self::Stale
    }

    fn wrong_side() -> Self {
        Self::Client
    }
}
