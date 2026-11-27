//! A server player's last death location for native mods, through a
//! `PlayerRef` (docs/modding/src/how-to/native-mods.md): `Player.getLastDeathLocation` and
//! `setLastDeathLocation`, as a `GlobalPos` (the dimension's key and the
//! block), so a death in a dimension that is not open still reads. It is kept
//! in the player's record (swapped into `level.last_death` for the player's
//! turn), saved as `LastDeathLocation`, and sent with their next respawn
//! packet, which is what the recovery compass follows. Players resolve
//! through the shared player-handle path (`native_server.rs`), so reads return
//! None for a stale handle, for a player who isn't online here or whose record
//! is swapped out, and when the player never died. A write needs a living
//! player (`native_as_player`), so it is refused inside `player_died`, where
//! the read still works.
use mod_api::{GlobalPos, Identifier, PlayerRef};
use sunrise_core::position::BlockPos;

pub trait NativePlayerDeathGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// The dimension and block of the player's last death
    /// (`Player.getLastDeathLocation`).
    fn player_last_death_location(&self, player: PlayerRef) -> Option<GlobalPos> {
        let (dimension, [x, y, z]) = self.native_player_last_death_record(player)?;
        let dimension = Identifier::parse(&dimension).ok()?;
        Some(GlobalPos::new(dimension, BlockPos::new(x, y, z)))
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativePlayerDeathGameExt1<'__sg> for G {}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use mod_api::{GlobalPos, Identifier};
    use sunrise_core::position::BlockPos;

    /// A block of the overworld.
    pub fn at([x, y, z]: [i32; 3]) -> GlobalPos {
        GlobalPos::new(
            Identifier::parse("minecraft:overworld").unwrap(),
            BlockPos::new(x, y, z),
        )
    }

    /// A placed guest record.
    pub fn new_guest(conn: u32, uuid: u128) -> sunrise_sim::lan::guest::Guest {
        use std::sync::{Arc, atomic::AtomicI32, mpsc};
        use sunrise_protocol::packets::configuration::ClientInformation;
        use sunrise_sim::lan::{Joined, PlayerSender, guest::Guest};

        let (tx, _rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn,
                uuid,
                name: "guest".into(),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            20,
            [0.0; 3],
            8,
        );
        guest.placed = true;
        guest
    }
}

use crate::game::native_server::NativeServerGameExt1 as _;
