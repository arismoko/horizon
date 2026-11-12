//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
/// The server host's methods of the `scoreboard` area, implemented by the game and reached through [`ServerHost`].
#[horizon_mod_api_macros::host_area]
pub trait ScoreboardServerHost {
    /// Side: server.
    /// `Scoreboard.addPlayerTeam` and `PlayerTeam.setColor`, as `/team add` and
    /// `/team modify <team> color` do: the team `name` is made when it is missing, and its
    /// colour becomes `color`, one of the sixteen chat colour names (`red`, `dark_aqua`...),
    /// or none for None (`reset`). Its members' names take the colour in every client from
    /// the next tick's scoreboard sync. The team's other settings stay. Teams are kept with
    /// the world's scoreboard, which the server saves. False on a client, for a name that is
    /// empty or has a space, and for an unknown colour (nothing changes then).
    #[access(WorldOnly)]
    fn team_set(&mut self, name: &str, color: Option<&str>) -> bool;
    /// Side: server.
    /// `Scoreboard.addPlayerToTeam`, as `/team join`: `holder` (a player's name, or an
    /// entity's UUID as the scoreboard keeps it) leaves its team and joins `team`, in every
    /// client from the next tick's sync. False on a client, for an empty holder and for a
    /// team that doesn't exist (nothing changes then).
    #[access(WorldOnly)]
    fn team_join(&mut self, team: &str, holder: &str) -> bool;
    /// Side: server.
    /// `Scoreboard.removePlayerFromTeam`, as `/team leave`: `holder` leaves the team it is on.
    /// False on a client and for a holder on no team.
    #[access(WorldOnly)]
    fn team_leave(&mut self, holder: &str) -> bool;
    /// Side: server.
    /// `Scoreboard.removePlayerTeam`, as `/team remove`: the team is gone and its members have
    /// no team. False on a client and for a team that doesn't exist.
    #[access(WorldOnly)]
    fn team_remove(&mut self, name: &str) -> bool;
}
