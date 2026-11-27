//! Server-side last death location write (`Player.setLastDeathLocation`).
use crate::game::ServerGame;
use mod_api::{GlobalPos, PlayerRef};

impl ServerGame<'_> {
    /// `Player.setLastDeathLocation`: stores where the player died, or
    /// forgets it for None. The client hears of it with the player's next
    /// respawn, as in vanilla. False in the cases a read gives None.
    pub(in crate::game) fn player_set_last_death_location(
        &mut self,
        player: PlayerRef,
        location: Option<GlobalPos>,
    ) -> bool {
        self.native_as_resident_player(player, |game| {
            game.level.last_death = location.map(|l| {
                let p = l.pos();
                (l.dimension().to_string(), [p.x, p.y, p.z])
            });
        })
    }
}
